//! The editor's library: its Lisp setup, command loop glue, fonts and terminal
//! service, which the program and the integration tests share. The process
//! itself (the event loop, the paint path, worlds, the client) is `main.rs''s.
pub mod command;
pub mod echo;
pub mod eval_last_sexp;
pub mod fonts;
pub mod grammars;
pub mod lisp_layout;
pub mod loading;
pub mod loadup;
pub mod measure;
pub mod native;
pub mod presentation;
pub mod startup;
pub mod stderr;
pub mod terminal_helper;
pub mod terminal_proto;
pub mod terminal_service;
pub mod terminal_wait;
pub mod watchdog;
#[cfg(test)]
#[path = "../tests/support/home.rs"]
mod test_home;

/// The arena as this library's unit tests' allocator, so a test's loadup
/// can restore a world (`test_world`), as the program's does.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: romacs_vm::world::arena::Arena = romacs_vm::world::arena::Arena;
#[cfg(test)]
mod test_world;
