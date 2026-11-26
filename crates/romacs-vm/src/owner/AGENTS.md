# crates/romacs-vm/src/owner
Owners: zygote children in the role `owner` that serve for the session,
holding processes whose filters run in their own loop. This module keeps
their liveness (the ping), an owner's own flag, and the windows it lays
out for its master (`display.rs`, under `lisp/vm/owner-window.el`).

## Boundary
- The frames (`Kind::Ping`, `Kind::Pong`), the control socket and the
  waits belong to `crates/romacs-vm/src/isolate.rs`; the waits that answer
  pings to it and to `crates/romacs-vm/src/quit.rs` (`poll_answering`).
- Lifecycle, calls, state and the options are Lisp's:
  `lisp/vm/owner.el`, `lisp/vm/isolate.el`, and the loop in
  `lisp/vm/isolate-worker.el`. Every change to an isolate's life goes
  through one pure transition and its edge,
  `lisp/vm/isolate-lifecycle.el`.

## Contracts
- Wedged means an unanswered ping is past its own deadline, the
  `owner-wedge-delay` it went under, nothing else. Pings go every half the delay while a call is outstanding
  or a ping is unanswered, so a lost ping or pong heals at the next; a
  pong answers its ping and every earlier one, and one for a ping never
  sent is no news.
- Every wait an owner makes answers pings in Rust and leaves every other
  frame queued for its turn. A turn blocked in a wait is never wedged.
- The loop takes the control socket before each batch of at most
  `owner-output-batch` filter and sentinel calls
  (`romacs--process-dispatches`); a wait inside one is not bounded.
- Killing an owner never waits: SIGTERM, then SIGKILL at a deadline that
  a later pump enforces. One its zygote has not reported yet gets the
  last signal asked once it is (`Vm::isolate_signal`).
- An interrupt's deadline kills no owner.
- An owner never serves without the features it was asked for: its
  zygote's failed preparation refuses it (`romacs--isolate-unprepared`).
- Owners count against `isolate-process-limit` when one is asked for; the
  pool's admission, its count and its room coming alike, leaves them out
  (`romacs--isolate-counted-p`), and no worker is ended to make an owner room.
- No callback or `isolate-error-functions` runs before every isolate a
  transition touched holds its next state: shutdown kills every owner first.
- A worker is its build: a hello naming another build than the master's
  is refused without a wait (`romacs--isolate-hello-check`); a fork names its
  zygote's, wherever the executable lies.
- A quit or error a filter, sentinel or timer lets out between calls ends
  that turn, never the owner (`romacs--isolate-run-turns`).

- Window layouts live with the host's display (`crates/romacs-vm/src/ui.rs`).

## Tests
- `python3 tools/dev-loop romacs-vm --profile gate -E 'test(/owner|isolate|quit_reach/)'`
  and the same with `--test all`: one process a test, since
  `crates/romacs-vm/tests/isolate_interrupt.rs` signals its own process.
- Windows and pictures: `romacs-vm-gui` test `all`, `owner_logical_windows`.

## Traps
- An owner dies with its zygote (its death signal). A termination quits
  its call with no reply: the death settles it (`isolate-died`). A retired
  zygote forks the children asked for before its bye, lingers while its
  owners live, and is not on its way out for admission
  (`romacs--isolate-ending-p`).
- Until the publication socket, pings and pongs ride the control socket and
  the master's evaluator reads the pongs, at its pumps.
