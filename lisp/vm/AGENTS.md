# lisp/vm
The VM's own Lisp: romacs-vm's preludes and the Lisp halves of its
primitives, which every VM reads from its runtime tree as the tree holds it
(`crates/romacs-vm/src/prims/mod.rs`), never from a copy compiled into the
executable.

## Contracts
- A file here and the Rust that calls it are one contract: the helpers a
  primitive calls by name, the variables it reads, the arguments and values
  they pass.
- An executable trusts the tree it runs over. A change here that needs
  matching Rust (a helper added, renamed or removed, an argument list or a
  value changed) raises `etc/romacs-runtime-epoch` in the same commit, so an
  executable refuses a tree of another generation instead of calling into
  Lisp that no longer fits it. Lisp that runs with the Rust as it is raises
  nothing. `tools/dev-check` names a change of this directory beside
  romacs-vm's Rust that leaves the epoch alone; it cannot tell which it is.

## Tests
- `cargo test -p romacs-vm --test vm_lisp_from_tree`: an edited file here is
  what a VM runs, and a process with no tree is told why.

## Traps
- A Lisp half is one `(progn ...)`: its primitive reads the file's first form
  only, so a second top-level form never runs. The preludes are evaluated
  whole.
- No `load-history` entry names these files: their definitions stand in for
  GNU's C.
