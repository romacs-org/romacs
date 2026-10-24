# Retired VM2 session contract

Source: `retired/romacs-core-2026-10-19`, `crates/romacs/tests/vm2_batch.rs` and `crates/romacs/src/vm2_session.rs`.

The surviving contract exercises `romacs_vm::Vm` directly: persistent evaluation requests, arithmetic, frame identity, the absence of the inherited recursive editor, formatted messages, first-error termination within a source request, and heap-backed session state across observed collections. The error test additionally checks that a failed request leaves the VM usable without executing later forms.

The tagged `vm2_session.rs` contains no unit tests. Its VM evaluation loop is the runtime behavior represented here; its argument parser is not retained as a second implementation.

Retired boundaries: shipped binary selection through `ROMACS_RUNTIME`, CLI flag aliases and parsing, rejecting unsupported or interactive invocations, whole-argv validation before evaluation, exit statuses, stdout/stderr routing, and comparison against the deleted core evaluator. In particular, stopping separate CLI `--eval` requests after an error was a host-loop policy; the retained test proves termination within `Vm::eval_source`, not that deleted policy. Message return values remain asserted, but exact process stderr routing is no longer an acceptance assertion.

Run after registering this directory as a workspace member:

```sh
cargo test -p romacs-retired-vm-contract
```
