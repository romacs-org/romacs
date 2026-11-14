//! The keeper: a small, long-lived process that holds a session's
//! descriptors and is its children's parent.
//!
//! A session process comes and goes: it is restarted, upgraded, and
//! sometimes it crashes. What must outlive it lives here. The keeper is a
//! descriptor store and a parent, and nothing else:
//!
//! - A **handle** is an id (counting up, never reused), a kind tag and
//!   version the keeper does not interpret, a set of descriptors,
//!   optionally a child the keeper is the parent of, and a small metadata
//!   blob its owner sets.
//! - The keeper **spawns** on request ([`spawn::Spec`] carries the child's
//!   whole start), keeps the descriptors open, **reaps**, and records each
//!   child's exit, stop and continue.
//! - It hands **duplicates** of a handle's descriptors to the session that
//!   owns it. Data never flows through the keeper: the session reads and
//!   writes its duplicates directly. When a session dies the keeper's
//!   copies keep the pipe, pty or socket open; the program blocks when the
//!   kernel's buffer fills, and a successor is handed duplicates again.
//!
//! So adding a kind of resource never changes the keeper. Native state
//! that must outlive a session (a terminal emulator, a TLS session) lives
//! in a helper process the keeper spawns like any child.
//!
//! # Rules
//!
//! - **One owner.** A session is the process at the other end of its
//!   connection, known by pidfd. Its exit is its detach, whatever became of
//!   the socket. It has one connection: a newer one replaces the older. A handle owned by a live session is refused to any other,
//!   by the owner's pid, unless the claimant names that pid as the session
//!   it supersedes; the keeper then kills it first, because a duplicate
//!   cannot be revoked.
//! - **Quit, restart, crash.** A session that quits detaches with `quit`
//!   and its handles close, unless marked kept. One that restarts or
//!   crashes leaves them. A deadline closes a handle that has no live
//!   owner once it has passed; a claim cancels it. No timeout of the keeper's own closes a handle:
//!   only a close, a deadline its owner set, or the successor's sweep of
//!   what nobody claimed ([`proto::kind::CLOSE_ORPHANS`]).
//! - **Closing is a request**, slot by slot: end of file reaches a child
//!   only when every holder of the write end has closed it. What end of
//!   file means for a pty or a socket is the session's business.
//! - **Signals go through the keeper**, by handle. It reaps, so a pid a
//!   session remembers may already name another process.
//! - **Status comes from the keeper**: only the parent can wait. The
//!   current status is in the handle's row for good; a change is also
//!   pushed to the owner as an event.
//! - **The keeper never reads or writes a kept descriptor**, so file
//!   status flags (`O_NONBLOCK`) are the session's alone.
//!
//! # Where children run
//!
//! In the keeper's process group setup, cgroup, resource limits and OOM
//! scope, not the session's. A child gets exactly the environment,
//! directory, umask, limits, signal mask and descriptors its spec names.
//! Its `getppid` is the keeper for good, across session restarts and across
//! the keeper's own replacement.
//!
//! # Replacing the keeper
//!
//! By exec in place ([`exec`]): same pid, same children, same unit. All of
//! its state is the [`table::Table`], so nothing else needs carrying.
//! Sessions hold their own duplicates and only reconnect.
//!
//! # One spawn mechanism, two hosts
//!
//! [`spawn`] is a library. A process that keeps no children (`--batch`, a
//! test, a worker VM) calls it in process and uses no keeper.

pub mod address;
pub mod client;
pub mod exec;
pub mod proto;
pub mod run;
pub mod server;
pub mod spawn;
pub mod table;

pub use client::{Client, Event};
pub use server::Build;
