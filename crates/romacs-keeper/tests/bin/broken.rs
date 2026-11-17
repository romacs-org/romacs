//! A keeper build that passes the table check and then cannot resume, for
//! the exec tests: the guarded exec must fall back to the old executable.
fn main() -> std::process::ExitCode {
    if std::env::args().any(|a| a == "--check-table") {
        return std::process::ExitCode::SUCCESS;
    }
    romacs_keeper::run::main(romacs_keeper::Build { table: 0 })
}
