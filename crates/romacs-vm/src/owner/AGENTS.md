# crates/romacs-vm/src/owner
Owners: zygote children in the role `owner` that serve for the session,
holding processes whose filters run in their own loop. This module keeps
their liveness (the ping) and an owner's own flag.

## Boundary
- The frames (`Kind::Ping`, `Kind::Pong`), the control socket and the
  waits belong to `crates/romacs-vm/src/isolate.rs`; the waits that answer
  pings to it and to `crates/romacs-vm/src/quit.rs` (`poll_answering`).
- Lifecycle, calls, state and the options are Lisp's:
  `lisp/vm/owner.el`, `lisp/vm/isolate.el`, and the loop in
  `lisp/vm/isolate-worker.el`.

## Contracts
- Wedged means the oldest unanswered ping is `owner-wedge-delay` old,
  nothing else. Pings go every half the delay while a call is outstanding
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

## Tests
- `python3 tools/dev-loop romacs-vm --profile gate -E 'test(/owner|isolate|quit_reach/)'`
  and the same with `--test all`: one process a test, since
  `crates/romacs-vm/tests/isolate_interrupt.rs` signals its own process.

## Traps
- An owner dies with its zygote (its death signal). A termination quits
  its call with no reply: the death settles it (`isolate-died`). A retired
  zygote forks the children asked for before its bye, lingers while its
  owners live, and is not on its way out for admission
  (`romacs--isolate-ending-p`).
- Until the publication socket, pings and pongs ride the control socket and
  the master's evaluator reads the pongs, at its pumps.
