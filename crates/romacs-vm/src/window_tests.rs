use super::*;
fn run(vm: &mut Vm, source: &str) -> Value {
    let result = vm.eval_source_raw(source);
    assert!(
        result.is_ok(),
        "window behavior must execute: {source}: {result:?}"
    );
    result.unwrap()
}
fn yes(vm: &mut Vm, source: &str) {
    assert_eq!(run(vm, source), Value::TRUE, "{source}");
}
#[test]
fn frame_identity_is_lazy_heap_owned() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0, "fresh VM allocation count must not change");
    let frame = run(&mut vm, "(selected-frame)");
    assert_eq!(frame.tag(), 0, "frame identity must be a heap object, not an interned symbol");
    yes(&mut vm, "(and (eq (selected-frame) (window-frame)) (eq (frame-selected-window) (selected-window)) (not (symbolp (selected-frame))))");
}

#[test]
fn frame_graph_edges_move_and_retain_constant_storage() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0);
    run(&mut vm, "(vector 0)"); // garbage forces real relocation
    run(&mut vm, SPLIT);
    let frame = vm.selected_frame_value().unwrap();
    let parameter = run(&mut vm, "(list (copy-sequence \"frame-only-owner\"))");
    vm.frame_mut(frame).unwrap().parameters.push((Value::TRUE, parameter));
    let terminal = vm.frame(frame).unwrap().terminal;
    let old_frame = frame;
    let old_terminal = terminal;
    let before = vm.heap_stats().collections;
    vm.collect(&mut []);
    let frame = vm.selected_frame_value().unwrap();
    assert_ne!(frame, old_frame, "frame fixture must move");
    let terminal = vm.frame(frame).unwrap().terminal;
    assert_ne!(terminal, old_terminal, "terminal fixture must move");
    assert_eq!(vm.heap_stats().collections, before + 1);
    let owner = vm.frame(frame).unwrap().parameters.iter().find(|(key, _)| *key == Value::TRUE).unwrap().1;
    assert_eq!(vm.display(owner).unwrap(), "(\"frame-only-owner\")", "frame parameters must be forwarded");
    assert_eq!(vm.window(vm.frame(frame).unwrap().selected).unwrap().frame, frame, "window backlink must be forwarded");
    let data::Object::Terminal(t) = vm.object(terminal).unwrap() else { panic!("terminal type") };
    assert_eq!(t.frame, frame, "terminal frame edge must be forwarded");
    let mut retained = vec![];
    for _ in 0..3 {
        vm.collect(&mut []);
        let s = vm.heap_stats();
        retained.push((s.live_objects, s.live_bytes));
    }
    eprintln!("frame retention {retained:?}");
    assert!(retained.iter().all(|s| *s == retained[0]), "frame graph retention grows");
    yes(&mut vm, "(and (eq (window-frame a) (selected-frame)) (eq (window-frame b) (selected-frame)))");
}

#[test]
fn frame_combination_limit_controls_parent_creation() {
    let mut vm = Vm::new();
    run(&mut vm, "(setq window-combination-limit t)");
    run(&mut vm, SPLIT);
    yes(&mut vm, "(eq (window-combination-limit (window-parent a)) t)");
    run(&mut vm, "(set-window-new-pixel a 115) (setq c (split-window-internal a 115 nil 0.5))");
    yes(&mut vm, "(not (eq (window-parent a) (window-parent b)))");
}

#[test]
fn frame_buffer_search_starts_after_selected_window() {
    let mut vm = Vm::new();
    run(&mut vm, SPLIT);
    run(&mut vm, "(set-window-new-pixel b 115) (setq c (split-window-internal b 115 nil 0.5)) (setq x (get-buffer-create \"cyclic\")) (set-window-buffer a x) (set-window-buffer c x) (select-window b)");
    yes(&mut vm, "(eq (get-buffer-window x) c)");
}

const SPLIT: &str = "(progn (setq a (selected-window)) (set-window-new-pixel a 230) (setq b (split-window-internal a 230 nil 0.5)))";
#[test]
fn g3a_lazy_heap_tree_and_minibuffer() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0);
    yes(
        &mut vm,
        "(progn (setq a (selected-window)) (and (windowp a) (window-live-p a) (not (eq a (minibuffer-window))) (eq a (frame-selected-window)) (= (window-total-height (minibuffer-window)) 1)))",
    );
    assert_eq!(
        run(&mut vm, "a").tag(),
        0,
        "window must be a heap object, not a symbol"
    );
}
#[test]
fn g3a_split_preparation_and_delete_rollback() {
    let mut vm = Vm::new();
    run(&mut vm, SPLIT);
    yes(
        &mut vm,
        "(and (window-live-p a) (window-live-p b) (eq (window-parent a) (window-parent b)) (= (window-total-height a) 11) (= (window-total-height b) 11))",
    );
    let error = vm.eval_source_raw("(delete-window-internal b)").unwrap_err();
    assert!(error.contains("Deletion failed"), "{error}");
    yes(
        &mut vm,
        "(and (eq (window-next-sibling a) b) (eq (window-prev-sibling b) a) (window-live-p b))",
    );
    run(
        &mut vm,
        "(progn (set-window-new-pixel (window-parent a) 460) (set-window-new-pixel a 460) (delete-window-internal b))",
    );
    yes(
        &mut vm,
        "(and (window-live-p a) (not (window-live-p b)) (not (window-parent a)) (= (window-pixel-height a) 460))",
    );
    assert_eq!(run(&mut vm, "(delete-window-internal b)"), Value::NIL);
}
#[test]
fn g3a_selection_and_independent_points() {
    let mut vm = Vm::new();
    run(
        &mut vm,
        "(progn (set-buffer (get-buffer-create \"points\")) (insert \"abcdef\") (set-window-buffer nil (current-buffer)))",
    );
    run(&mut vm, SPLIT);
    yes(
        &mut vm,
        "(progn (set-window-point a 2) (set-window-point b 5) (select-window b) (and (= (point) 5) (= (window-point a) 2) (eq (current-buffer) (window-buffer b))))",
    );
    yes(
        &mut vm,
        "(progn (select-window a) (and (= (point) 2) (= (window-point b) 5) (= (set-window-point a 900) 900) (= (window-point a) 7)))",
    );
}
/// `set-window-buffer` must take the buffer's own point, not position 1.
///
/// GNU sets the window's point marker from `BUF_PT (b)` and skips the whole
/// reset when KEEP-MARGINS is non-nil and the buffer is unchanged
/// (`src/window.c:4367-4378`, the `!(keep_margins_p && samebuf)` guard, with
/// `samebuf` computed before the buffer is assigned). Verified on live GNU
/// 32.0.50: with "abcdef" at point 7, assigning that buffer to an unselected
/// window leaves `window-point` at 7, and selecting the window puts point at 7.
///
/// The older point test sets window points *after* assignment, so it cannot see
/// an assignment that resets them.
#[test]
fn g3a_set_window_buffer_takes_the_buffers_point() {
    let mut vm = Vm::new();
    run(&mut vm, SPLIT);
    // Buffer point is 7 (end of "abcdef"); `b` is not the selected window.
    run(
        &mut vm,
        "(progn (set-buffer (get-buffer-create \"carried\")) (erase-buffer) (insert \"abcdef\") (goto-char 7))",
    );
    yes(
        &mut vm,
        "(progn (set-window-buffer b (get-buffer \"carried\")) (= (window-point b) 7))",
    );
    // Selecting it must land point where the window says, not at 1.
    yes(&mut vm, "(progn (select-window b) (= (point) 7))");

    // KEEP-MARGINS with an unchanged buffer must not disturb the markers.
    run(&mut vm, "(select-window a)");
    yes(&mut vm, "(progn (set-window-point b 3) (= (window-point b) 3))");
    yes(
        &mut vm,
        "(progn (set-window-buffer b (get-buffer \"carried\") t) (= (window-point b) 3))",
    );
    // Without KEEP-MARGINS, the same assignment re-reads the buffer's point.
    yes(
        &mut vm,
        "(progn (set-buffer (get-buffer \"carried\")) (goto-char 5) (set-window-buffer b (get-buffer \"carried\")) (= (window-point b) 5))",
    );
}

#[test]
fn g3a_parameters_eq_dead_and_gc() {
    let mut vm = Vm::new();
    run(&mut vm, "(vector 99)");
    run(&mut vm, SPLIT);
    yes(
        &mut vm,
        "(progn (setq k (copy-sequence \"key\")) (setq v (list \"payload\")) (set-window-parameter b k v) (and (eq (window-parameter b k) v) (not (window-parameter b (copy-sequence \"key\")))))",
    );
    run(
        &mut vm,
        "(progn (vector 99) (set-window-parameter b 'only-owner (list (copy-sequence \"gc-only\"))))",
    );
    let before = vm.heap_stats().collections;
    let old_window = run(&mut vm, "b");
    vm.collect(&mut []);
    assert_eq!(vm.heap_stats().collections, before + 1);
    assert_ne!(old_window, run(&mut vm, "b"), "window fixture must move");
    yes(
        &mut vm,
        "(and (eq (window-parameter b k) v) (equal (window-parameter b 'only-owner) '(\"gc-only\")))",
    );
    yes(
        &mut vm,
        "(progn (set-window-parameter b k nil) (and (assq k (window-parameters b)) (not (window-parameter b k))))",
    );
    run(
        &mut vm,
        "(progn (set-window-new-pixel (window-parent a) 460) (set-window-new-pixel a 460) (delete-window-internal b) (set-window-parameter b 'dead 'yes))",
    );
    yes(&mut vm, "(eq (window-parameter b 'dead) 'yes)");
    assert!(vm.eval_source_raw("(window-parameters b)").is_err());
}
#[test]
fn g3a_margins_start_and_stale_end() {
    let mut vm = Vm::new();
    run(
        &mut vm,
        "(progn (set-buffer (get-buffer-create \"scroll\")) (insert \"abcdef\") (set-window-buffer nil (current-buffer)))",
    );
    yes(
        &mut vm,
        "(and (set-window-margins nil 2 3) (not (set-window-margins nil 2 3)) (not (set-window-margins nil 999 3)) (equal (window-margins) '(2 . 3)) (= (set-window-start nil 3) 3) (= (window-start) 3))",
    );
    // Before a host lays the frame out, UPDATE keeps the recorded end, as
    // GNU's batch does.
    yes(&mut vm, "(= (window-end nil t) (window-end))");
    // NOFORCE sets the start without forcing it on redisplay.
    yes(&mut vm, "(and (= (set-window-start nil 2 t) 2) (= (window-start) 2))");
    assert_eq!(run(&mut vm, "(set-window-start nil nil)"), Value::NIL);
}
#[test]
fn g3a_resize_axis_and_nested_tree() {
    let mut vm = Vm::new();
    run(&mut vm, SPLIT);
    run(
        &mut vm,
        "(progn (set-window-new-pixel b 320) (setq c (split-window-internal b 320 t 0.5)))",
    );
    yes(
        &mut vm,
        "(and (= (window-pixel-width b) 320) (= (window-pixel-width c) 320) (= (window-pixel-height c) 230) (eq (window-parent (window-parent c)) (window-parent a)))",
    );
    assert!(
        vm.eval_source_raw("(split-window-internal c 80 t 0.5)")
            .is_err()
    );
    yes(
        &mut vm,
        "(progn (set-window-new-pixel (window-parent a) 460) (set-window-new-pixel a 200) (set-window-new-pixel (window-parent b) 260) (set-window-new-pixel b 260) (set-window-new-pixel c 260) (window-resize-apply nil nil))",
    );
    yes(
        &mut vm,
        "(and (= (window-pixel-height a) 200) (= (window-pixel-height b) 260) (= (window-total-height c 'ceiling) 13))",
    );
    run(
        &mut vm,
        "(progn (set-window-new-pixel b 130) (setq d (split-window-internal b 130 nil 0.5)))",
    );
    run(
        &mut vm,
        "(progn (set-window-new-pixel (frame-root-window) 640) (set-window-new-pixel a 640) (set-window-new-pixel (window-parent c) 640) (set-window-new-pixel (window-parent b) 640) (set-window-new-pixel b 640) (set-window-new-pixel d 640) (delete-window-internal c))",
    );
    yes(
        &mut vm,
        "(and (eq (window-parent a) (window-parent b)) (eq (window-parent b) (window-parent d)) (eq (window-next-sibling b) d) (= (window-pixel-width b) 640))",
    );
}
#[test]
fn g3a_collections_and_three_size_retention() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    run(&mut vm, "(selected-window)");
    let mut retained = vec![];
    let mut symbol_counts = vec![];
    for size in [16, 64, 256] {
        for _ in 0..size {
            run(&mut vm, SPLIT);
            run(
                &mut vm,
                "(progn (set-window-parameter b 'payload (vector 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9 9)) (set-window-new-pixel (window-parent a) 460) (set-window-new-pixel a 460) (delete-window-internal b) (setq b nil))",
            );
        }
        vm.collect(&mut []);
        let s = vm.heap_stats();
        eprintln!(
            "g3a size={size} collections={} objects={} bytes={} capacity={}",
            s.collections, s.live_objects, s.live_bytes, s.capacity_bytes
        );
        retained.push((s.live_objects, s.live_bytes));
        symbol_counts.push(vm.symbol_names.len());
    }
    assert_eq!(vm.heap_stats().collections, 3);
    assert_eq!(retained[0], retained[1]);
    assert_eq!(retained[1], retained[2]);
    assert_eq!(
        symbol_counts[0], symbol_counts[1],
        "window identities must not intern permanent symbols"
    );
    assert_eq!(symbol_counts[1], symbol_counts[2]);
    yes(&mut vm, "(window-live-p (selected-window))");
}
/// A parent that *survives* must not keep the peak storage of its children.
///
/// `window.rs` drops children with `Vec::retain`, which removes elements but
/// does not reduce capacity, and the collector forwards a surviving window
/// without compacting its vectors. So growing one parent to 4, then 8, then 16
/// siblings and deleting them again leaves identical topology and identical
/// object counts while retained bytes climb with the peak.
///
/// `g3a_collections_and_three_size_retention` cannot see this: it splits and
/// deletes in each iteration, so the parent is created and retired every time
/// and never survives holding an oversized child vector.
///
/// Every eval here uses a **fixed** source string that computes its own sizes
/// from the live window, and the handles live in one list variable. Both details
/// are load-bearing: a `format!`-generated source per iteration allocates that
/// source's own constants -- the `0.5` float alone showed up as `peak + 1`
/// retained floats -- and binding `w0..wN` interns a symbol per child. Either
/// one reads exactly like a leak in the window code and is purely the test's
/// own doing. Sizes stay in Lisp because `delete-window-internal` applies
/// caller-prepared sizes and signals otherwise (src/window.c:5530), and VM2 has
/// no `window-list`.
#[test]
fn g3a_surviving_parent_does_not_retain_peak_child_storage() {
    // One fixed source per operation, so no per-iteration constants.
    const GROW: &str = "(progn \
        (set-window-new-pixel a (- (window-pixel-height a) 64)) \
        (setq ws (cons (split-window-internal a 64 nil 0.5) ws)) \
        (set-window-parameter (car ws) 'payload (vector 9 9 9 9 9 9 9 9)))";
    const SHRINK: &str = "(progn \
        (set-window-new-pixel (window-parent a) 2048) \
        (set-window-new-pixel a (+ (window-pixel-height a) 64)) \
        (delete-window-internal (car ws)) (setq ws (cdr ws)))";
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 2048 + 24, 24, 12).unwrap();
    vm.set_collection_floor(1_000_000);
    run(&mut vm, "(setq a (selected-window) ws nil)");
    // A keeper child, so the parent combination exists before and after every
    // measurement and is never retired.
    run(&mut vm, GROW);
    run(&mut vm, "(setq keep (car ws) ws (cdr ws))");

    // The window tree's OWN storage must not keep the peak: `children` is a
    // plain Vec, so `retain` leaves the allocation behind and a surviving
    // parent would hold it for life. Measured across ascending peaks because
    // that is exactly what a retained peak allocation shows.
    let window_storage = |vm: &Vm| -> usize {
        vm.objects
            .iter()
            .filter_map(|o| match o {
                crate::data::Object::Window(w) => Some(w.storage_bytes()),
                _ => None,
            })
            .sum()
    };
    let mut per_peak = vec![];
    for peak in [4, 8, 16] {
        for _ in 0..peak {
            run(&mut vm, GROW);
        }
        for _ in 0..peak {
            run(&mut vm, SHRINK);
        }
        vm.collect(&mut []);
        per_peak.push(window_storage(&vm));
    }
    assert!(
        per_peak.iter().all(|b| *b == per_peak[0]),
        "a surviving parent retained peak child storage: {per_peak:?}"
    );

    // And the heap as a whole must not grow when the SAME workload repeats.
    //
    // Repetition, not ascending peaks, is the right property here. The buffer's
    // anchor table is a free-list allocator: `anchors::remove` detaches a slot
    // and pushes it on `self.free` for reuse, while `storage_bytes` reports
    // `self.n.len()`, the high-water mark. Measured: repeating peak 16 three
    // times holds 5664 bytes exactly, and running 16 then 8 then 4 also holds
    // 5664 -- the mark is set by the largest peak and never exceeded. Growth
    // across *ascending* peaks is therefore correct high-water-mark behaviour,
    // like `Vec::capacity`, and asserting flatness there would be demanding
    // that a recycling allocator shrink. A real leak grows on repetition, which
    // this does not.
    crate::gc::assert_no_retention_growth(&mut vm, &[16, 16, 16], |vm, peak| {
        for _ in 0..peak {
            run(vm, GROW);
        }
        for _ in 0..peak {
            run(vm, SHRINK);
        }
    });
    yes(&mut vm, "(and (window-live-p (selected-window)) (window-live-p keep))");
}

/// Publishing one window must not invalidate another window's last publication.
///
/// `display_state.previous` held a single shared snapshot, so within one
/// `frame_snapshot` the two windows clobbered each other's record and every
/// frame republished as changed, repainting the whole frame for nothing.
///
/// Selection deliberately does NOT change here. An earlier version of this test
/// alternated `select-window` and demanded no repaint, which was wrong: GNU
/// `window.c:544-551` marks both the previously selected and the newly selected
/// window for redisplay on `select-window` and then calls
/// `redisplay_other_windows`, because the mode line switches between the active
/// and inactive face. VM2 bumping its version for `SelectWindow` is parity, so
/// only a test that holds selection fixed can see the shared-snapshot defect.
#[test]
fn g3a_publishing_one_window_does_not_repaint_another() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    run(&mut vm, "(setq a (selected-window))");
    run(&mut vm, "(insert \"alpha\")");
    // A second window on a DIFFERENT buffer, so the two snapshots differ.
    run(
        &mut vm,
        "(progn (set-window-new-pixel a 288) \
           (setq b (split-window-internal a 288 nil 0.5)) \
           (set-window-buffer b (get-buffer-create (generate-new-buffer-name \"other\"))))",
    );
    // Settle: every window gets its own first publication.
    vm.frame_snapshot(800, 576).unwrap();
    vm.frame_snapshot(800, 576).unwrap();
    let settled = vm.display_version();

    // Republish the identical frame. Nothing has changed anywhere.
    vm.frame_snapshot(800, 576).unwrap();
    vm.frame_snapshot(800, 576).unwrap();

    assert_eq!(
        vm.display_version(),
        settled,
        "republishing an unchanged two-window frame bumped the display version, \
         so each window invalidated the other and the frame repaints for nothing"
    );
}

/// A split child inherits the reference window's margins.
///
/// GNU copies them explicitly from the reference window to the new one at
/// `window.c:5611-5612`, `n->left_margin_cols = r->left_margin_cols` and the
/// right margin beside it, in the `if (!dead)` block that carries over the
/// reference window's "special geometry settings". VM2's split copied x, y,
/// width, height and new_pixel but left the child's margins at their zero
/// defaults, so the child silently rendered a wider body than GNU would give
/// it.
///
/// Queried immediately after the split, before anything can reset them: the
/// existing layout scene sets margins only after splitting and then reassigns
/// the child's buffer, which resets margins anyway, so it could not see this.
#[test]
fn g3a_split_child_inherits_reference_window_margins() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    run(&mut vm, "(setq a (selected-window))");
    run(&mut vm, "(set-window-margins a 7 3)");
    run(
        &mut vm,
        "(progn (set-window-new-pixel a 288) \
           (setq b (split-window-internal a 288 nil 0.5)))",
    );
    // GNU returns (left . right) with nil for a zero-width margin
    // (window.c:8531-8534), so an uninherited margin reads as (nil . nil).
    yes(&mut vm, "(equal (window-margins b) '(7 . 3))");
    // The reference window keeps its own, and both sides stay independent.
    yes(&mut vm, "(equal (window-margins a) '(7 . 3))");
    run(&mut vm, "(set-window-margins b 1 nil)");
    yes(&mut vm, "(equal (window-margins b) '(1))");
    yes(&mut vm, "(equal (window-margins a) '(7 . 3))");
}

/// Retiring a subtree must drop every descendant's published snapshot.
///
/// `retire_window` recurses into children, but pruning ran only for the one
/// window named in `delete-window-internal`. Deleting an internal parent
/// therefore killed its descendant leaves while their snapshots stayed in the
/// publication map, and because window ids are monotonic and never reused the
/// map grew for the life of the VM, holding owned text with it.
///
/// Asserted on the map directly. The map is Rust-side state, not a Lisp heap
/// object, so `gc::assert_no_retention_growth` and every heap-object or
/// live-byte count are structurally incapable of seeing this leak.
#[test]
fn g3a_retiring_a_subtree_drops_every_descendant_publication() {
    // The second split is HORIZONTAL on purpose: a same-orientation split joins
    // the existing combination, so `inner`'s parent would be the root and
    // `delete-window-internal` refuses it ("Attempt to delete minibuffer or sole
    // ordinary window"). Differing orientation builds a real nested combination
    // whose retirement recurses into two leaves.
    const BUILD: &str = "(progn \
        (set-window-new-pixel a (- (window-pixel-height a) 200)) \
        (setq inner (split-window-internal a 200 nil 0.5)) \
        (set-window-new-pixel inner 400) \
        (setq leaf (split-window-internal inner 400 t 0.5)))";
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    run(&mut vm, "(setq a (selected-window))");
    let baseline = {
        run(&mut vm, BUILD);
        vm.frame_snapshot(800, 576).unwrap();
        let published = vm.published_window_count();
        // Delete the internal parent: `inner` and `leaf` both retire.
        run(&mut vm, "(progn (setq total (window-pixel-height (window-parent a))) \
                       (set-window-new-pixel (window-parent a) total) \
                       (set-window-new-pixel a total) \
                       (delete-window-internal (window-parent inner)) \
                       (setq inner nil leaf nil))");
        vm.frame_snapshot(800, 576).unwrap();
        assert!(published > vm.published_window_count(),
            "the subtree's publications were never dropped: {published} then {}",
            vm.published_window_count());
        vm.published_window_count()
    };
    // Repeat: a leak accumulates, a correct prune does not.
    for _ in 0..4 {
        run(&mut vm, BUILD);
        vm.frame_snapshot(800, 576).unwrap();
        run(&mut vm, "(progn (setq total (window-pixel-height (window-parent a))) \
                       (set-window-new-pixel (window-parent a) total) \
                       (set-window-new-pixel a total) \
                       (delete-window-internal (window-parent inner)) \
                       (setq inner nil leaf nil))");
        vm.frame_snapshot(800, 576).unwrap();
    }
    assert_eq!(vm.published_window_count(), baseline,
        "published snapshots accumulate across repeated subtree deletion");
}

#[test]
fn g3a_internal_split_uses_selected_reference_margins() {
    let mut vm = Vm::new();
    run(&mut vm, SPLIT);
    run(
        &mut vm,
        "(progn (set-window-margins a 1 2) (set-window-margins b 7 3) (select-window b) (setq parent (window-parent a)) (set-window-new-pixel parent 320) (set-window-new-pixel a 320) (set-window-new-pixel b 320) (setq child (split-window-internal parent 320 t 0.5)))",
    );
    let margins = run(&mut vm, "(window-margins child)");
    assert_eq!(vm.display(margins).unwrap(), "(7 . 3)");
    vm.collect(&mut []);
    yes(&mut vm, "(equal (window-margins child) '(7 . 3))");
    run(&mut vm, "(set-window-margins child 4 5)");
    yes(&mut vm, "(equal (window-margins b) '(7 . 3))");
}

#[test]
fn g3a_buffer_assignment_resets_recorded_end_distance() {
    for keep in ["nil", "t"] {
        let mut vm = Vm::new();
        run(
            &mut vm,
            r#"(progn (set-buffer (get-buffer-create "recorded-end")) (insert "Ωabcdef") (set-window-buffer nil (current-buffer)))"#,
        );
        let frame = vm.frame_snapshot(640, 480).unwrap();
        vm.display_apply_layout(&[crate::WindowEnd { id: frame.windows[0].snapshot.id, end: 4, visible_end: 0, body_height: 0.0, rows: vec![], point: None }])
            .unwrap();
        assert_eq!(run(&mut vm, "(window-end)").as_integer().unwrap(), 4);
        run(
            &mut vm,
            &format!(r#"(set-window-buffer nil (get-buffer-create "empty-end") {keep})"#),
        );
        assert_eq!(
            run(&mut vm, "(window-end)").as_integer().unwrap(),
            1,
            "buffer assignment must discard the previous end distance, KEEP-MARGINS={keep}"
        );
    }
}

#[test]
fn g3a_geometry_and_layout_end_journal() {
    let mut vm = Vm::new();
    let geometry = vm.set_frame_geometry(800, 600, 24, 12);
    assert!(geometry.is_ok(), "geometry must be installed: {geometry:?}");
    yes(
        &mut vm,
        "(and (= (window-pixel-height) 576) (= (window-total-height) 24) (= (window-total-height (minibuffer-window)) 1) (= (window-pixel-width) 800))",
    );
    run(
        &mut vm,
        "(progn (set-buffer (get-buffer-create \"ends\")) (insert \"Ωabcdef\") (set-window-buffer nil (current-buffer)))",
    );
    let snapshot = vm.display_snapshot(800, 576).unwrap();
    vm.display_apply_layout(&[crate::WindowEnd { id: snapshot.id, end: 4, visible_end: 0, body_height: 0.0, rows: vec![], point: None }]).unwrap();
    yes(&mut vm, "(= (window-end nil) 4)");
    // A laid-out window lays itself out again for UPDATE, which needs the
    // host's layout attached.
    let refused = r#"(equal (condition-case e (window-end nil t) (error (car (cdr e)))) "VM2 window-end in a displayed window is unsupported")"#;
    yes(&mut vm, refused);
    run(&mut vm, "(insert \"z\")");
    yes(&mut vm, refused);
    yes(&mut vm, "(= (window-end nil) 5)");
    assert!(vm.set_frame_geometry(1, 1, 24, 12).is_err());
}
#[test]
fn g3a_owned_frame_scene_from_window_tree() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(640, 480, 20, 10).unwrap();
    run(&mut vm, SPLIT);
    run(
        &mut vm,
        r#"(progn
      (set-buffer (get-buffer-create "top")) (insert "Ω top") (set-window-buffer a (current-buffer))
      (set-buffer (get-buffer-create "bottom")) (insert "bottom text") (set-window-buffer b (current-buffer))
      (set-window-point b 4) (set-window-start b 2) (set-window-margins b 2 3))"#,
    );
    let frame = vm.frame_window_snapshots(640, 480);
    assert!(
        frame.is_ok(),
        "frame must consume the VM window tree: {frame:?}"
    );
    let frame = frame.unwrap();
    assert_eq!(frame.len(), 3, "two leaves and reserved minibuffer");
    assert_eq!(
        (frame[0].0, frame[0].1, frame[0].2.width, frame[0].2.height),
        (0, 0, 640, 230)
    );
    assert_eq!(
        (frame[1].0, frame[1].1, frame[1].2.width, frame[1].2.height),
        (20, 230, 590, 230)
    );
    assert_eq!((frame[2].0, frame[2].1, frame[2].2.height), (0, 460, 20));
    assert_eq!(frame[0].2.text.utf8, "Ω top");
    assert_eq!(frame[1].2.text.utf8, "bottom text");
    assert_eq!(frame[1].2.start_byte, 1);
    assert_eq!(frame[1].2.point_byte, 3);
    assert!(frame[2].2.mode_line.is_none());
    assert_ne!(frame[0].2.id, frame[1].2.id);
    let face = vm.display_default_face().unwrap();
    assert_eq!(face.background, [0, 0, 0]);
    assert!(face.pixel_size > 0.0);
    vm.collect(&mut []);
    assert_eq!(frame[0].2.text.utf8, "Ω top");
    let after = vm.frame_window_snapshots(640, 480).unwrap();
    for (a, b) in frame.iter().zip(after.iter()) {
        assert_eq!(
            (a.0, a.1, a.2.id, a.2.text.clone()),
            (b.0, b.1, b.2.id, b.2.text.clone())
        );
    }
    yes(
        &mut vm,
        "(and (= (frame-pixel-width) 640) (= (frame-pixel-height) 480))",
    );
    run(
        &mut vm,
        r#"(setq mode-line-format '((:eval (let ((i 0) (garbage nil)) (while (< i 100) (setq garbage (cons i garbage) i (1+ i))) "mode"))))"#,
    );
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    let collected = vm.frame_window_snapshots(640, 480).unwrap();
    assert!(
        vm.heap_stats().collections > before,
        "mode-line evaluation must really collect"
    );
    assert_eq!(collected[0].2.text.utf8, "Ω top");
    assert_eq!(collected[1].2.text.utf8, "bottom text");
}

/// vscroll is per-window state, not frame state.
///
/// GNU's `window-vscroll` and `set-window-vscroll` both begin with
/// `decode_live_window (window)` and then read or write that window's own
/// `w->vscroll` (`src/window.c:8947` and `:8978` onward). VM2 kept a single
/// `vscroll_px` on `DisplayState` and published it into every window, so setting
/// one window's vscroll was observable from a different, untouched window.
///
/// This is the third finding of one shape: frame-wide state in `DisplayState`
/// consumed per window. The shared `previous` snapshot and the frame-wide
/// `width`/`height` were the first two. The margins never had this bug because
/// they live on the window, which is where this belongs too.
#[test]
fn g3a_vscroll_is_owned_by_each_window_not_shared_across_the_frame() {
    fn vscroll_of(vm: &mut Vm, w: &str) -> Result<i64> {
        let v = vm.eval_source_raw(&format!("(window-vscroll {w} t)"))?;
        v.integer_value().ok_or_else(|| "an integer vscroll".to_string())
    }
    let mut vm = Vm::new();
    run(&mut vm, SPLIT);
    run(&mut vm, "(set-window-vscroll a 7 t)");
    assert_eq!(vscroll_of(&mut vm, "a"), Ok(7),
               "the window that was scrolled reports its own vscroll");
    // GNU decodes the WINDOW argument (decode_live_window, window.c:8947) and
    // reads that window's own w->vscroll. B was never scrolled.
    assert_eq!(vscroll_of(&mut vm, "b"), Ok(0),
               "an untouched live window must be decodable and must not observe \
                another window's vscroll");
    // The same leak must not arrive via selection.
    run(&mut vm, "(select-window b)");
    assert_eq!(vscroll_of(&mut vm, "nil"), Ok(0),
               "the selected untouched window must not inherit the other's vscroll");
    // Independent in both directions.
    run(&mut vm, "(set-window-vscroll b 3 t)");
    assert_eq!(vscroll_of(&mut vm, "a"), Ok(7), "scrolling B must not disturb A");
    assert_eq!(vscroll_of(&mut vm, "b"), Ok(3), "B keeps its own value");
}

/// GNU redisplay's shortcut for a window nothing it shows changed in: a
/// redisplay where only its cursor was turned off, or its point moved within
/// the rows the last layout showed, keeps its text, faces and start, walks
/// and fontifies no text (try_cursor_movement), and evaluates only its mode
/// line; an edit or an overlay change redisplays it fully.
#[test]
fn unchanged_windows_redisplay_only_their_cursor_and_mode_line() {
    let mut vm = Vm::new();
    // A collection counts as a change (`ShownKey'): no forced ones.
    vm.set_gc_stress(0);
    // `romacs--redisplay-fontify', the walk over unfontified text.
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    run(&mut vm, r#"(progn (setq walks 0 evals 0) (set-buffer (get-buffer-create "shown"))
                          (dotimes (i 10) (insert (format "line %d\n" i)))
                          (set-window-buffer nil (current-buffer)) (goto-char 1)
                          ;; Never marks text fontified: called by every full redisplay.
                          (setq fontification-functions (list (lambda (_) (setq walks (1+ walks)))))
                          (setq mode-line-format '("%l " (:eval (progn (setq evals (1+ evals)) "x")))))"#);
    let snapshot = |vm: &mut Vm| vm.frame_snapshot(800, 576).unwrap().windows[0].snapshot.clone();
    let counts = |vm: &mut Vm| (run(vm, "walks").as_integer().unwrap(), run(vm, "evals").as_integer().unwrap());
    let first = snapshot(&mut vm);
    // The host showed the whole buffer; the first hosted redisplay is full.
    vm.display_apply_layout(&[whole(&first)]).unwrap();
    // As the GUI does after each paint: the rows reuse rests on.
    let full = snapshot(&mut vm);
    vm.display_apply_layout(&[whole(&full)]).unwrap();
    let (walks, evals) = counts(&mut vm);
    assert!(walks > 0);

    run(&mut vm, "(internal-show-cursor nil nil)");
    let blinked = snapshot(&mut vm);
    assert!(blinked.cursor_off && blinked.lays_out_like(&first), "only the cursor changed");
    assert_eq!(counts(&mut vm), (walks, evals + 1), "a blink walked the text");
    run(&mut vm, "(internal-show-cursor nil t)");
    assert!(!snapshot(&mut vm).cursor_off);

    run(&mut vm, "(forward-line 3)");
    let moved = snapshot(&mut vm);
    assert_eq!(counts(&mut vm), (walks, evals + 3), "a moved point walked the text");
    assert_eq!((moved.start_byte, &moved.text), (first.start_byte, &first.text));
    assert_eq!(moved.point_byte, first.text.utf8.find("line 3").unwrap());
    assert_eq!(moved.mode_line.as_ref().unwrap().utf8, "4 x");

    run(&mut vm, "(insert \"new \")");
    let edited = snapshot(&mut vm);
    assert!(edited.text.utf8.contains("new line 3"), "an edit redisplays the text");
    assert!(counts(&mut vm).0 > walks, "an edit walks the text");
    vm.display_apply_layout(&[whole(&edited)]).unwrap();
    snapshot(&mut vm);
    let (walks, _) = counts(&mut vm);
    run(&mut vm, "(overlay-put (make-overlay 1 3) 'face 'bold)");
    snapshot(&mut vm);
    assert!(counts(&mut vm).0 > walks, "an overlay change walks the text");
    let (walks, _) = counts(&mut vm);
    run(&mut vm, "(narrow-to-region 1 20)");
    snapshot(&mut vm);
    assert!(counts(&mut vm).0 > walks, "narrowing walks the text");
    run(&mut vm, "(widen)");
    snapshot(&mut vm);
    let (walks, _) = counts(&mut vm);
    snapshot(&mut vm);
    assert_eq!(counts(&mut vm).0, walks, "and then nothing changed");
}

/// A mode line whose `:eval' collects during a redisplay that reuses the
/// window's snapshot moves the window: the redisplay goes on with the
/// window's traced handle, not the one it read before the Lisp ran.
#[test]
fn a_collection_in_a_reused_windows_mode_line_moves_nothing_it_reads() {
    let mut vm = Vm::new();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    // Garbage below the window the redisplay reuses, so a collection moves it.
    run(&mut vm, r#"(progn (setq junk (list (make-string 9 ?j) (make-vector 9 0) (make-string 9 ?k)))
                          (set-window-new-pixel (selected-window) 288)
                          (setq lower (split-window-internal (selected-window) 288 nil 0.5))
                          ;; Strings just above it: where the window's old index points after the move.
                          (setq keep (let (l) (dotimes (_ 20) (push (make-string 1 ?z) l)) l))
                          (select-window lower)
                          (set-buffer (get-buffer-create "moved")) (insert "a\nb\nc\n") (set-window-buffer nil (current-buffer))
                          (setq collect nil)
                          (set (make-local-variable 'mode-line-format) '((:eval (progn (if collect (garbage-collect)) "m")))))"#);
    let frame = vm.frame_snapshot(800, 576).unwrap();
    let ends: Vec<_> = frame.windows.iter().map(|w| whole(&w.snapshot)).collect();
    vm.display_apply_layout(&ends).unwrap();
    vm.frame_snapshot(800, 576).unwrap();
    // Dead only now: the reused redisplay's collection moves the window down.
    run(&mut vm, "(setq junk nil collect t)");
    let before = vm.heap_stats().collections;
    let again = vm.frame_snapshot(800, 576).expect("a redisplay that collected");
    assert!(vm.heap_stats().collections > before, "the mode line collected");
    assert!(again.windows.iter().any(|w| w.snapshot.text.utf8 == "a\nb\nc\n"));
}

/// Lisp a reused window's mode line runs can change faces: a face setter
/// drops every window's last publication and a new face drops the reuse
/// records. Either makes that redisplay a full one, which publishes the
/// changed faces, instead of the reused snapshot's old ones or a panic.
#[test]
fn face_changes_in_a_reused_windows_mode_line_redisplay_it_fully() {
    for (change, color) in [("(internal-set-lisp-face-attribute 'reuse-face :foreground \"#ff0000\" nil)", [255, 0, 0]),
                            ("(defface reuse-new '((t (:foreground \"#00ff00\"))) \"n\")", [0, 255, 0])] {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.set_frame_geometry(800, 600, 24, 12).unwrap();
        run(&mut vm, &format!(r##"(progn (defface reuse-face '((t (:foreground "#0000ff"))) "r")
                                       (set-buffer (get-buffer-create "faces")) (insert (propertize "abc" 'face 'reuse-face) "\nd\ne\n")
                                       (set-window-buffer nil (current-buffer)) (setq change nil)
                                       (setq mode-line-format '((:eval (progn (when change (setq change nil) {change}) "m")))))"##));
        let first = vm.frame_snapshot(800, 576).unwrap().windows[0].snapshot.clone();
        vm.display_apply_layout(&[whole(&first)]).unwrap();
        vm.frame_snapshot(800, 576).unwrap();
        run(&mut vm, "(setq change t)");
        let changed = vm.frame_snapshot(800, 576).expect(change).windows[0].snapshot.clone();
        assert!(changed.faces.iter().any(|face| face.foreground == color), "{change} did not reach the published faces");
    }
}

/// The header line takes the mode line's shortcut: a redisplay that keeps
/// a window's text, faces and start evaluates its header line again, as
/// display_mode_lines builds both bands; a header line the window starts
/// to want (its `header-line-format' parameter) takes rows from the text,
/// so it redisplays the window fully.
#[test]
fn unchanged_windows_redisplay_their_header_line_too() {
    let mut vm = Vm::new();
    vm.set_gc_stress(0);
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    run(&mut vm, r#"(progn (setq walks 0 heads 0) (set-buffer (get-buffer-create "headed"))
                          (dotimes (i 10) (insert (format "line %d\n" i)))
                          (set-window-buffer nil (current-buffer)) (goto-char 1)
                          (setq fontification-functions (list (lambda (_) (setq walks (1+ walks)))))
                          (setq mode-line-format "%l")
                          (setq header-line-format '("H%l " (:eval (progn (setq heads (1+ heads)) "h")))))"#);
    let snapshot = |vm: &mut Vm| vm.frame_snapshot(800, 576).unwrap().windows[0].snapshot.clone();
    let counts = |vm: &mut Vm| (run(vm, "walks").as_integer().unwrap(), run(vm, "heads").as_integer().unwrap());
    let header = |snapshot: &romacs_layout_input::WindowSnapshot| snapshot.header_line.as_ref().map(|band| band.utf8.clone());
    let first = snapshot(&mut vm);
    vm.display_apply_layout(&[whole(&first)]).unwrap();
    let full = snapshot(&mut vm);
    vm.display_apply_layout(&[whole(&full)]).unwrap();
    assert_eq!(header(&full).as_deref(), Some("H1 h"));
    let (walks, heads) = counts(&mut vm);

    run(&mut vm, "(forward-line 3)");
    let moved = snapshot(&mut vm);
    assert_eq!(counts(&mut vm), (walks, heads + 1), "a moved point walked the text, or kept the header line");
    assert_eq!(header(&moved).as_deref(), Some("H4 h"));
    assert_eq!(moved.mode_line.as_ref().unwrap().utf8, "4");

    run(&mut vm, "(progn (setq header-line-format nil) (force-window-update))");
    let bare = snapshot(&mut vm);
    assert_eq!(header(&bare), None);
    vm.display_apply_layout(&[whole(&bare)]).unwrap();
    snapshot(&mut vm);
    let (walks, _) = counts(&mut vm);
    run(&mut vm, "(set-window-parameter nil 'header-line-format \"P\")");
    let parameter = snapshot(&mut vm);
    assert_eq!(header(&parameter).as_deref(), Some("P"));
    assert!(counts(&mut vm).0 > walks, "a header line the window now wants kept the text");
}

/// The layout journal of a host that showed all of SNAPSHOT's text on
/// whole rows.
fn whole(snapshot: &romacs_layout_input::WindowSnapshot) -> crate::WindowEnd {
    let len = snapshot.text.utf8.len();
    crate::WindowEnd { id: snapshot.id, end: len, visible_end: len + 1, body_height: snapshot.height as f32, rows: vec![], point: None }
}

/// Overlay calls that change nothing a window may show leave its record
/// standing, as GNU's buffer.c calls no modify_overlay for them: a new
/// overlay (no properties yet), deleting a detached one, and `overlay-put'
/// of a value `eq' to the one there. A real change still redisplays fully.
#[test]
fn overlay_calls_that_change_nothing_keep_the_window() {
    let mut vm = Vm::new();
    vm.set_gc_stress(0);
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 24, 12).unwrap();
    run(&mut vm, r#"(progn (setq walks 0) (set-buffer (get-buffer-create "kept"))
                          (dotimes (i 10) (insert (format "line %d\n" i)))
                          (set-window-buffer nil (current-buffer)) (goto-char 1)
                          (setq fontification-functions (list (lambda (_) (setq walks (1+ walks)))))
                          (setq mode-line-format '("%l")))"#);
    // Each frame painted, as the GUI does: the record cursor moves reuse.
    let mut frame = |vm: &mut Vm| {
        let snapshot = vm.frame_snapshot(800, 576).unwrap().windows[0].snapshot.clone();
        vm.display_apply_layout(&[whole(&snapshot)]).unwrap();
    };
    let walks = |vm: &mut Vm| run(vm, "walks").as_integer().unwrap();
    frame(&mut vm);
    frame(&mut vm);
    let before = walks(&mut vm);
    for (form, changes) in [("(setq o (make-overlay 1 3))", false), ("(overlay-put o 'face nil)", false),
                            ("(forward-char 1)", false), ("(overlay-put o 'face 'bold)", true),
                            ("(delete-overlay o)", true), ("(delete-overlay o)", false)] {
        let before = walks(&mut vm);
        run(&mut vm, form);
        frame(&mut vm);
        assert_eq!(walks(&mut vm) > before, changes, "{form}");
    }
    assert!(walks(&mut vm) > before);
}
