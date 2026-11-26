# crates/romacs-vm/src/command_loop
GNU keyboard.c's key reader (`read_key_sequence`, `read_char`,
`read_filtered_event`) and the input stream it reads into (GNU's KBOARD).

## Boundary
- Owns the reader and the stream's command keys, single-command start and
  raw keys. The command loop's step, hooks, input methods, filters,
  translation functions, the host and batch waits and the echo stay Lisp
  (`lisp/vm/keyboard.el`): the reader calls them, never reimplements them.
- Keymap lookups go through C1w's walk (`crates/romacs-vm/src/keymap/walk.rs`)
  and the active maps through `crates/romacs-vm/src/active_keymaps.rs`.

## Contracts
- Orchestration in the `crate::brand` style: no `Value` in a Rust local
  across a Lisp call. Each reading primitive pushes an activation
  (`stream::Act`) with its own result, pending write, walk and
  `KeyseqFrame`, traced and rewritten by the collector, so a read nested in
  its Lisp neither overwrites nor drains it.
- A leaf makes at most one write to a watched variable, as its last effect,
  in the Lisp port's order; `step` completes it before anything else runs.
- `Err(())` means an escape waits in `propagated_escape`. Every read leaves
  through `keyseq::read`'s exit: the end hooks once if the sequence started
  (the escape kept unless they make their own), then its frame popped.
- `Streams::store` is the only writer of a stream's fields (Rust privacy is
  the census of DESIGN §2.1).
- Follow maps are taken at GNU's three points only. `test_undefined` and the
  final remap reuse them only while no Lisp ran since and the first event
  had no position.
- A change to the Lisp helpers the reader calls by name raises
  `etc/romacs-runtime-epoch`.

## Tests
- `cargo test --profile gate --locked --offline -p romacs-vm --lib command_loop::`
- `python3 tools/test-ert --file test/romacs/lisp/command-loop-tests.el`, and
  the same file on the pinned GNU.

## Traps
- `(t . E)` and `(no-record . E)` are decoded twice: by `read_char` for the
  queues, and by `romacs--read-host-input` for an event a timer queued during
  the host wait (beta's rows).

## Decisions
- The reads are a field of their own, at rest (`Vm::reads`), not inside
  `InputStream`: only an image's root may hold at-rest fields.
- Writes to watched variables are requests right after the leaf that made
  them (`store`, `flush`), so their watchers run where the Lisp reader's
  `setq` ran them.
