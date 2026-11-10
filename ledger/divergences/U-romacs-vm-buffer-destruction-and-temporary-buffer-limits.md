## Buffer destruction and temporary-buffer limits

Explicit `kill-buffer` runs query functions and kill hooks with the target
current, unwinds current-buffer/point on veto or error, and destroys native
registry identity, buffer-local storage, windows, markers and overlays on
success. The buffer object remains a dead `bufferp` identity across collection.
`tests/kill_buffer.rs` covers those paths, including collection inside callbacks.

This is not complete GNU buffer lifecycle parity. The current `with-temp-buffer`
expansion marks its temporary buffer dead and removes its name on exit, but uses
an internal cleanup path without query functions, kill hooks, window replacement,
or attachment and local-table destruction. Anonymous internal buffers have no
registered name and are absent from `buffer-list`.
`buffer-list` uses creation-slot order (not GNU MRU/frame order) and refuses a
non-nil FRAME. Window replacement chooses the earliest surviving visible named
buffer outside minibuffer windows, or creates a named scratch buffer; it does
not restore per-window history.
Killing the GUI minibuffer is refused to protect its echo storage. There is no
process termination, indirect-buffer cascade, autosave deletion, lock handling,
`buffer-list-update-hook`, or inhibit-buffer-hooks creation argument yet.
Unibyte excursion restoration preserves the numeric point because its marker
substrate is not implemented; Unicode restoration uses a moving marker.

For example GNU Emacs 32.0.50 prints `[nil nil nil nil]` below, but VM2's current
temporary-buffer expansion retains the attachments until explicitly killed:

```elisp
(let ((m (with-temp-buffer (insert "abc") (copy-marker 2)))
      (o (with-temp-buffer (insert "abc") (make-overlay 1 3))))
  (vector (marker-position m) (marker-buffer m) (overlay-start o) (overlay-end o)))
```
