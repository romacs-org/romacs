use std::fs; // expect: std::fs use-host

fn planted(p: &Path, dir: &Path, file: &File) {
    std::fs::write(p, b""); // expect: std::fs
    let saved = vm.dynamic[0].1; // expect: dynamic:saved
    vm.slot("user-mail-address"); // expect: delayed-variable
    for entry in &vm.dynamic {} // expect: dynamic:saved
    File::open(p); // expect: File::open
    std::fs::File::create(p); // expect: File::create
    std::os::unix::fs::symlink(a, b); // expect: std::fs
    use std::os::unix::fs::{MetadataExt, symlink}; // expect: std::fs use-host
    OpenOptions::new(); // expect: OpenOptions
    dir.read_dir(); // expect: read_dir
    file.metadata(); // expect: metadata
    p.symlink_metadata(); // expect: symlink_metadata
    p.canonicalize(); // expect: canonicalize
    Path::new(p).exists(); // expect: Path::exists
    p.is_file(); // expect: Path::is_file
    p.is_dir(); // expect: Path::is_dir
    std::path::absolute(p); // expect: std::path::absolute
    std::env::var("A"); // expect: std::env::var
    env::var_os("A"); // expect: std::env::var_os
    std::env::vars(); // expect: std::env::vars
    std::env::vars_os(); // expect: std::env::vars_os
    std::env::current_dir(); // expect: std::env::current_dir
    std::env::current_exe(); // expect: std::env::current_exe
    std::env::temp_dir(); // expect: std::env::temp_dir
    std::process::id(); // expect: std::process::id
    SystemTime::now(); // expect: SystemTime::now
    std::time::Instant::now(); // expect: Instant::now
    std::thread::available_parallelism(); // expect: available_parallelism
    libc::getpwuid(0); // expect: libc::getpw*
    libc::getgrgid_r(0); // expect: libc::getgr*
    libc::getpid(); // expect: libc::getpid
    libc::getppid(); // expect: libc::getppid
    libc::getuid(); // expect: libc::getuid
    libc::geteuid(); // expect: libc::geteuid
    libc::getgid(); // expect: libc::getgid
    libc::getegid(); // expect: libc::getegid
    libc::gethostname(b, n); // expect: libc::gethostname
    libc::uname(u); // expect: libc::uname
    libc::sysconf(n); // expect: libc::sysconf
    libc::sched_getaffinity(0, n, s); // expect: libc::sched_getaffinity
    libc::statvfs(p, s); // expect: libc::statvfs
    libc::clock_gettime(c, t); // expect: libc::clock
    libc::getrandom(b, n, 0); // expect: libc::getrandom
    std::fs::read_dir(p); // expect: std::fs read_dir
    vm.slot("process-environment"); // expect: slot:process-environment
    vm.slot("initial-environment"); // expect: slot:initial-environment
    vm.slot("exec-path"); // expect: slot:exec-path
    Forwarded::lookup("default-directory"); // expect: slot:default-directory
    p.try_exists(); // expect: Path::try_exists
    p.is_symlink(); // expect: Path::is_symlink
    p.read_link(); // expect: Path::read_link
    std::env::set_var("A", "b"); // expect: std::env::set
    std::process::Command::new("true"); // expect: Command::new
    std::net::TcpStream::connect(addr); // expect: std::net
    ("localhost", 80).to_socket_addrs(); // expect: std::net
    libc::clock_gettime(c, t); // expect: libc::clock
    libc::getenv(n); // expect: libc::getenv
    libc::fchmodat(d, p, m, 0); // expect: libc::file
    libc::mkostemp(t, 0); // expect: libc::file
    libc::kill(pid, 0); // expect: libc::process
    libc::syscall(n); // expect: libc::process
    libc::pipe2(fds, 0); // expect: libc::descriptor
    libc::socket(f, t, 0); // expect: libc::socket
    rustix::process::getuid(); // expect: rustix::query
    rustix::fs::openat(d, p, f, m); // expect: rustix::file
    rustix::process::kill_process(pid, s); // expect: rustix::process
    rustix::pipe::pipe(); // expect: rustix::descriptor
    rustix::fs::fcntl_getfl(fd); // a descriptor call: no host query
}

// rustix's host modules are spelled in full.
use rustix::fs::{openat}; // expect: rustix::file use-host
use rustix::net as network; // expect: use-alias use-host

// Ordinary spellings that name a forbidden item another way.
use std::env::{var}; // expect: std::env::var use-host
fn imported() { var("PATH"); }
use std::time::Instant as Clock; // expect: use-alias use-host
fn aliased() { Clock::now(); }
fn qualified(p: &Path) {
    Path::is_file(p); // expect: Path::is_file
    PathBuf::is_dir(p); // expect: Path::is_dir
    paths.filter(Path::exists); // expect: Path::exists
    <Path>::canonicalize(p); // expect: canonicalize
    <std::time::Instant>::now(); // expect: Instant::now
    <File>::open(p); // expect: File::open
    std::env:: var("A"); // expect: std::env::var
    std::env:: // expect: std::env::var_os
        var_os("A");
}
use std::{
    process::id, // expect: std::process::id use-host
    env as environment, // expect: use-alias use-host
};
use std::process::id as pid; // expect: std::process::id use-alias use-host
use std::{self as s}; // expect: use-alias use-host
use libc::*; // expect: use-glob
use crate::shim::libc::*; // expect: use-glob
use libc::{getuid, c_int}; // expect: libc::getuid
pub use std::fs::read as slurp; // expect: std::fs use-alias use-host
extern crate libc as c; // expect: use-alias
type P = std::path::Path; // expect: type-alias
type Clock2 = Instant; // expect: type-alias
type Clock3<const N: usize> = std::time::Instant; // expect: type-alias
fn generic() { <Clock3<0>>::now(); }
// A host module imported plainly has a local name a later `use` could
// rename, so the plain import is the violation.
use std::env; // expect: use-host
use env as e;
fn chained() { e::var_os("HOME"); }
mod imports {
    use std::env; // expect: use-host
    use std::time; // expect: use-host
    use std::os::unix::fs::{self as _, MetadataExt}; // expect: std::fs use-host
    use std::fs::{File, OpenOptions}; // expect: std::fs OpenOptions use-host
    use File as Handle; // expect: use-alias
    use std::time::{Duration, Instant}; // expect: use-host
    use std::process::Stdio; // expect: use-host
    use std::path::Path;
    use Path as Probe; // expect: use-alias
}
fn through_other_modules() {
    ghost::read_dir(p); // expect: read_dir
    ghost::libc::kill(p, 0); // expect: libc::process
    host::libc::getuid(); // expect: libc::getuid
}

#[cfg(test)]
const SIZES: [u8; 2] = [0; 2];
fn after_a_test_item() { std::process::id(); } // expect: std::process::id

#[cfg(not(test))]
fn live_unless_testing() { std::process::id(); } // expect: std::process::id

// A test-only member hides itself and nothing after it.
struct Counted {
    seen: usize,
    #[cfg(test)]
    pub(crate) visits: Vec<std::fs::File>,
}

fn after_a_test_field(seen: usize) -> Counted {
    std::fs::read("a"); // expect: std::fs
    let counted = Counted { seen, #[cfg(test)] visits: vec![std::fs::File::open("b").unwrap()] };
    std::fs::read("c"); // expect: std::fs
    match seen {
        #[cfg(test)]
        0 => drop(std::fs::read("d")),
        _ => drop(std::fs::read("e")), // expect: std::fs
    }
    #[cfg(test)]
    std::fs::read("f");
    std::fs::read("g"); // expect: std::fs
    counted
}
