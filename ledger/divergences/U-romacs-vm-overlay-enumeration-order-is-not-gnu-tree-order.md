## Overlay enumeration order is not GNU tree order

Live GNU Emacs 32.0.50, isolated with `lane_emacs` (`env -i`, `-Q`, throwaway HOME), returns the following. These forms reverse the returned list while collecting IDs; the distinction affects both enumeration APIs, not property-priority selection. Romacs order is deterministic for the same operation history, but is not GNU order.

```elisp
(with-temp-buffer (insert "abcdefgh") (overlay-put (make-overlay 2 5) (quote id) 0) (overlay-put (make-overlay 2 5) (quote id) 1) (overlay-put (make-overlay 1 6) (quote id) 2) (overlay-put (make-overlay 3 4) (quote id) 3) (overlay-put (make-overlay 2 5) (quote id) 4) (let ((l (overlays-at 3)) (r nil)) (while l (setq r (cons (overlay-get (car l) (quote id)) r)) (setq l (cdr l))) r))
```

```text
GNU: (3 0 1 4 2)
Romacs: (3 4 1 0 2)
```

```elisp
(with-temp-buffer (insert "abcdefgh") (overlay-put (make-overlay 2 5) (quote id) 0) (overlay-put (make-overlay 2 5) (quote id) 1) (overlay-put (make-overlay 1 6) (quote id) 2) (overlay-put (make-overlay 3 4) (quote id) 3) (overlay-put (make-overlay 2 5) (quote id) 4) (let ((l (overlays-in 1 9)) (r nil)) (while l (setq r (cons (overlay-get (car l) (quote id)) r)) (setq l (cdr l))) r))
```

```text
GNU: (3 0 1 4 2)
Romacs: (3 4 1 0 2)
```

`overlays-at` with SORTED, character properties and overlay faces order
overlays by GNU's `compare_overlays` (buffer.c), which is not transitive, so
its input is GNU's too: ascending start, equal starts latest inserted first
(itree_insert_node; `move-overlay` reinserts when the start moves), merged
as glibc's qsort does (`src/overlay_order.rs`). An edit that brings two
overlay starts together keeps Romacs's insertion order where GNU keeps its
tree's. Where two overlays tie on priority, secondary priority and range,
GNU orders them by object address and Romacs by heap index; both follow
creation order for overlays made in sequence
(`crates/romacs-vm-gui/tests/gnu_arity_fixes.rs` against pinned GNU).
