# lisp/vm
The VM's own Lisp: romacs-vm's preludes and the Lisp halves of its
primitives, which every VM reads from its runtime tree as the tree holds it
(`crates/romacs-vm/src/prims/mod.rs`), never from a copy compiled into the
executable.

## Contracts
- A file here and the Rust that calls it are one contract: the helpers a
  primitive calls by name, the variables it reads, the arguments and values
  they pass.
- If a Lisp/Rust contract changes, add a file to
  `etc/romacs-runtime-epoch.d` in the same commit. Use a unique
  YYYY-MM-DD-change-name and one line giving the reason. The executable
  and runtime tree identify their generation by the sorted fragment names.
  Lisp that still runs with the existing Rust needs no fragment.

## Tests
- `cargo test -p romacs-vm --test vm_lisp_from_tree`: an edited file here is
  what a VM runs, and a process with no tree is told why.

## Traps
- A Lisp half is one `(progn ...)`: its primitive reads the file's first form
  only, so a second top-level form never runs. The preludes are evaluated
  whole.
- No `load-history` entry names the preludes or the halves: their
  definitions stand in for GNU's C.
- A prelude that signals fails `install_lisp_prelude` with its file named,
  and a half that signals fails its primitive at each call: an edit here can
  break every VM made over the tree.
