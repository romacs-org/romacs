//! A stand-in for the next keeper build, for the exec tests: it reads and
//! writes a table one version newer, with one more field per entry.
fn main() -> std::process::ExitCode {
    romacs_keeper::run::main(romacs_keeper::Build { table: romacs_keeper::table::TABLE_VERSION + 1 })
}
