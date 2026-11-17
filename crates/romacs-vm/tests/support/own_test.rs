//! A test that runs again in a child process names itself to the child's
//! `--exact`. Its name depends on the executable: the file is the crate
//! root of a test target of its own, and a module of the `all` target.

/// The name libtest gives the test function NAME of the module MODULE, the
/// caller's `module_path!()`: the module's path less its crate, then NAME.
/// So a test of `tests/FILE.rs` is NAME in its own target and `FILE::NAME`
/// in `all`.
pub fn name(module: &str, name: &str) -> String {
    match module.split_once("::") {
        Some((_crate, path)) => format!("{path}::{name}"),
        None => name.to_owned(),
    }
}

/// A child given a name that selects nothing runs no test and exits 0, so a
/// wrong name would pass every test that runs itself. This one asks the
/// executable it is in to list what the name selects: exactly itself.
#[test]
fn own_test_name_selects_exactly_this_test() {
    let own = name(module_path!(), "own_test_name_selects_exactly_this_test");
    let listed = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--list", "--exact", &own])
        .output()
        .unwrap();
    assert!(listed.status.success(), "--list failed: {listed:?}");
    assert_eq!(String::from_utf8_lossy(&listed.stdout).lines().filter(|line| line.ends_with(": test")).collect::<Vec<_>>(),
               [format!("{own}: test")]);
}
