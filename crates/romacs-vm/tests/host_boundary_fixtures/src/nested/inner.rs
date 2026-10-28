//! Reached through an inline module.
fn inner() { std::env::current_exe(); } // expect: std::env::current_exe
