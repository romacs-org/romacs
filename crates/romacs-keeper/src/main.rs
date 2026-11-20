#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    romacs_keeper::run::main(romacs_keeper::Build::default())
}
