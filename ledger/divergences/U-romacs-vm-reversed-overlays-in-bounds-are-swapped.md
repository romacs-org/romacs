## Reversed overlays-in bounds are swapped

GNU returns no overlays for reversed arguments; Romacs swaps the endpoints. The last two vector entries below expose this existing difference. Live isolated GNU Emacs 32.0.50 transcript:

```elisp
(with-temp-buffer (insert "abcdefgh") (overlay-put (make-overlay 1 9) (quote id) 0) (overlay-put (make-overlay 9 9) (quote id) 1) (overlay-put (make-overlay 5 5) (quote id) 2) (vector (length (overlays-in 9 9)) (length (overlays-in 8 9)) (length (overlays-in 1 9)) (length (overlays-in 5 5)) (length (overlays-in 9 5)) (length (overlays-in 5 1))))
```

```text
GNU: [1 2 3 2 0 0]
Romacs: [1 2 3 2 3 1]
```
