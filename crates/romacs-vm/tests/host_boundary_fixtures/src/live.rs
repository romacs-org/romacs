use std::fs; // expect: std::fs

fn planted(p: &Path, dir: &Path, file: &File) {
    std::fs::write(p, b""); // expect: std::fs
    File::open(p); // expect: File::open
    std::fs::File::create(p); // expect: File::create
    std::os::unix::fs::symlink(a, b); // expect: std::fs
    use std::os::unix::fs::{MetadataExt, symlink}; // expect: std::fs
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
    libc::clock_gettime(c, t); // expect: libc::clock_gettime
    libc::getrandom(b, n, 0); // expect: libc::getrandom
    std::fs::read_dir(p); // expect: std::fs read_dir
    vm.slot("process-environment"); // expect: slot:process-environment
    vm.slot("initial-environment"); // expect: slot:initial-environment
    vm.slot("exec-path"); // expect: slot:exec-path
    self.forwarded.lookup("default-directory"); // expect: slot:default-directory
}

#[cfg(test)]
const SIZES: [u8; 2] = [0; 2];
fn after_a_test_item() { std::process::id(); } // expect: std::process::id

#[cfg(not(test))]
fn live_unless_testing() { std::process::id(); } // expect: std::process::id
