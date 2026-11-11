# Architecture

Romacs is a Lisp machine emulator in Rust whose runtime is Emacs Lisp. Rust
provides object memory and collection, the evaluator and compiler, loading,
processes, and the display and input host; Lisp, GNU's and Romacs's own,
runs on it and carries commands and policy. It is a GPU GUI program plus
`--batch`; there is no TTY frontend.

## Bird's eye view

A key event reaches the binary (`crates/romacs-vm-gui`), which hands it to
the VM's command loop. The VM runs Lisp, edits buffers and windows, and at
the end of the command publishes a frame snapshot. The binary's paint step
lays the snapshot out with `romacs-layout` and hands the scene to
`romacs-display-runtime`, whose thread draws it with `romacs-renderer-wgpu`.
Lisp work that can leave the main VM runs in worker VMs (isolates) over
frozen snapshots.

## Where things are

| Looking for | Look in |
|---|---|
| The binary: GUI, `--batch`, the paint loop | `crates/romacs-vm-gui/src/main.rs` |
| Startup, loadup and the world image | `crates/romacs-vm-gui/src/startup.rs`, `loadup.rs`, `world.rs` |
| One Lisp machine (`romacs_vm::Vm`) | `crates/romacs-vm/src/lib.rs` |
| The reader | `crates/romacs-vm/src/reader/` |
| The interpreter and its dispatch | `crates/romacs-vm/src/threaded.rs` |
| Garbage collection | `crates/romacs-vm/src/gc.rs`, `gc_mark.rs`; rooting types in `crates/romacs-brand` |
| Primitives (`defsubr!`) | `crates/romacs-vm/src/prims/`, one file per subject |
| Buffers, text and insertion | `crates/romacs-vm/src/buffer.rs`, `text.rs`, `insdel.rs` |
| Windows, frames, the mode line | `crates/romacs-vm/src/window.rs`, `frame.rs`, `mode_line.rs` |
| Keymaps and the command loop | `crates/romacs-vm/src/keymap.rs`, `command.rs` |
| Regular expressions | `crates/romacs-regex` (GNU's regex-emacs.c, no evaluator) |
| Syntax tables and scanning | `crates/romacs-vm/src/syntax_scan.rs`, `syntax_index.rs` |
| Processes and timers | `crates/romacs-vm/src/subprocess.rs`, `crates/romacs-vm-gui/src/timers.rs` |
| Worker VMs (isolates) | `crates/romacs-vm/src/isolate.rs`, `crates/romacs-vm-gui/src/isolate_worker.rs` |
| Tree-sitter | `crates/romacs-vm/src/treesit/`, grammars in `crates/romacs-grammars` |
| What the VM and display exchange | `crates/romacs-display-protocol` |
| Layout | `crates/romacs-layout-input` (owned input), `crates/romacs-layout` (the pass) |
| Faces, themes, fonts | `crates/romacs-face`, `crates/romacs-font`, `crates/romacs-font-materializer` |
| Drawing | `crates/romacs-display-runtime` (the render thread), `romacs-renderer-wgpu`, `romacs-scene` |
| Images and video | `crates/romacs-imaging`, `crates/romacs-video`, `crates/romacs-video-model` |
| Agents (MCP relay, sessions) | `crates/romacs-mcp`, `lisp/romacs/romacs-agent*.el` |
| GNU's Lisp, pinned | `lisp/gnu/` (pin: `ledger/lisp-pin.json`) |
| Romacs's own Lisp and bundled packages | `lisp/romacs/`; bundled roots in `lisp/bundled.json` |
| What a session loads, in order | `lisp/romacs/romacs-loadup-bootstrap.el` |
| Runtime data Lisp reads (`data-directory`) | `etc/` |
| Fonts, icons, desktop entry, branding | `assets/` |
| GNU's test suite, pinned, and its runner | `gnu-tests/` (pin and exclusions in `ledger/`) |
| Romacs's Lisp test suites and benchmarks | `test/` (`make check`) |
| Scripts and tools, with their tests | `tools/` (commands), `tools/lib/` (modules), `tools/tests/` |
| Decisions checks enforce | `ledger/`: pins, smell and unsafe floors, censuses, the parity ratchet, `divergences/` |
| Where Romacs differs from GNU, and why | `ledger/divergences/<ID>.md` |
| Packaging | `nix/`, `flake.nix`, `tools/package-release` |
| Documentation | `doc/` |

## Invariants

- `romacs-vm` reaches the display only through `romacs-display-protocol`,
  the snapshot types of `romacs-layout-input` and the host traits; it depends
  on no renderer, layout engine, font, video or webview crate.
- `romacs-display-protocol`, `romacs-layout-input`, `romacs-layout`,
  `romacs-face`, `romacs-font` and `romacs-regex` hold no evaluator and are
  tested without a VM.
- GNU's files under `lisp/gnu/` and `gnu-tests/test/` are byte-identical to
  the pin, less its declared exclusions; `tools/check-derived` verifies it.
- A deliberate difference from GNU is one file in `ledger/divergences/`.

## Cross-cutting concerns

- **Checks:** `tools/check-derived` (derived files, pins, the smell and unsafe
  ratchets, the Python tests), `make gate-small` and `make gate-full` (Rust),
  `make check` (Lisp). AGENTS.md says how to work here.
- **Unsafe:** a `// SAFETY:` comment on every site, counted against
  `ledger/unsafe-floors.json`.
- **Smells:** counted against `ledger/smell-floors.json`, including this
  directory's top-level entries.
- **GNU:** a source of behaviour, not of architecture.
