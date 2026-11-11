## An overlays-in end beyond point-max errors

GNU clamps an end beyond the full buffer extent; Romacs rejects it with `index out of range`. This error is not silent truncation. After the deletion below, point-max is 8, while the query ends at 10. Live isolated GNU Emacs 32.0.50 transcript:

```elisp
(with-temp-buffer (insert "abcdefghi") (let ((o (make-overlay 2 8))) (overlay-put o (quote evaporate) t) (remove-overlays 4 6) (delete-region 2 4) (vector (overlay-start o) (length (overlays-in 1 10)))))
```

```text
GNU: [4 1]
Romacs: vm2: index out of range
```
