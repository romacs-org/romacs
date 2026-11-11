## Remapping

GNU's NORMAL/REMAP merge points are distinct. A REMAP hides earlier NORMAL
points **of the same name**; when another remap would repeat that REMAP,
the original face definition is used instead. We preserve this transitively:
`((default solaire-face))` where solaire inherits default, and the dotted
`((mode-line . mode-line-inactive))`, both work. The old rejection of the
inheritance-backreference fixture was a bug and is removed.

A cycle **between distinct remapping entries** (e.g. `((a b) (b a))`) is
still deliberately rejected; GNU falls back/truncates at its merge points.
At the buffer boundary use `View::bind_or_unremapped`: a rejected remapping
returns the unremapped, paintable theme **and** its diagnostic, for this
buffer only. The caller must report that diagnostic. `bind` remains the
strict Result-returning API. `FaceCache` caches valid bound views and faces;
it does not hide binding errors or choose the caller's diagnostic UI.

Tests: `readiness.rs` valid remaps, barrier mutation, fallback;
`cycles.rs` pure remapping-entry cycle; `remap_parity.rs` C-path fixtures.
