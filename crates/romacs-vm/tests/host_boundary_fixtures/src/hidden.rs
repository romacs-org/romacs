//! Declared under `#[cfg(test)]`: never scanned.
fn t() { std::fs::read(p); std::env::current_dir(); libc::getuid(); }
