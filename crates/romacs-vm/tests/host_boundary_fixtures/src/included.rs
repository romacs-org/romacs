//! Reached through `include!`.
fn included() { std::env::temp_dir(); } // expect: std::env::temp_dir
