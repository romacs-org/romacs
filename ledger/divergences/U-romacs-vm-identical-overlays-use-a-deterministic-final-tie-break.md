## Identical overlays use a deterministic final tie-break

```
identical-overlays-newer-winner=B
```

At equal priority, start and end, Romacs chooses the newer overlay identity. The
rule survives interval-tree traversal order and moving GC. GNU resolves the
final tie by object address: passing this form directly to `--eval` returned 3,
whereas `(prin1 (eval (read SOURCE) t))` returned 1 on the review host:

```elisp
(with-temp-buffer (insert "abc")
  (let ((a (make-overlay 1 4)) (b (make-overlay 1 4)) (c (make-overlay 1 4)))
    (overlay-put a 'face 1) (overlay-put b 'face 2) (overlay-put c 'face 3)
    (get-char-property 2 'face)))
```

An allocator address is not a compatibility oracle. This case is excluded from
live-GNU equality replay and retained as a Romacs-only determinism/compaction test.
GNU's implementation order is not the portable contract. Overlay endpoints belong to the shared marker/overlay anchor store,
never to text-property runs.
