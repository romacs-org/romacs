//! The host class census (L1-0): a child of
//! `host_boundary_lint`, whose scanner lists its rows (`file_scan`) and reads
//! the HOST comments that class them.
use super::{BOUNDARY, BOUNDARY_MODULES, CLASSES, CRATES, Row, Tag, fences_state, file_scan, known, repository, roots, scan};
use std::collections::{BTreeMap, BTreeSet};

/// The faults of ROWS and the classes their HOST COMMENTS name: none, unknown,
/// two, `pure` with an `effect(`, or a class above nothing. A host.rs function is
/// classed above its item, a descriptor operation above its line, and an
/// exception site outside host.rs by its own comment.
fn census(rows: &[Row], comments: &[Tag]) -> Vec<(String, Row)> {
    let mut faults = Vec::new();
    let mut used = BTreeSet::new();
    let classify = |row: &Row, tags: &[&Tag], faults: &mut Vec<(String, Row)>| {
        let items: Vec<&str> = tags.iter().flat_map(|t| t.apis.iter().map(String::as_str)).collect();
        let classes: Vec<&str> = items.iter().copied().filter(|a| CLASSES.contains(a)).collect();
        let unknown: Vec<&str> = items.iter().copied().filter(|a| !CLASSES.contains(a) && !known(a) && !fences_state(a)).collect();
        let fault = if !unknown.is_empty() {
            Some(format!("unknown class `{}`", unknown.join("`, `")))
        } else if classes.is_empty() {
            Some("unclassed".to_owned())
        } else if classes.len() > 1 {
            Some(format!("two classes `{}`", classes.join("`, `")))
        } else if classes == ["pure"] && row.effect {
            Some("pure, but its body calls effect(".to_owned())
        } else {
            None
        };
        faults.extend(fault.map(|f| (f, row.clone())));
    };
    for row in rows {
        let boundary = row.path == BOUNDARY;
        let above: Vec<&Tag> = comments.iter().filter(|t| t.path == row.path
            && if boundary { t.item == Some(row.line) } else { t.covers == Some(row.line) }).collect();
        used.extend(above.iter().map(|t| (t.path.clone(), t.line)));
        classify(row, &above, &mut faults);
    }
    for site in exception_sites(comments) {
        used.extend(site.iter().map(|t| (t.path.clone(), t.line)));
        let what = site.iter().map(|t| t.apis.join(", ")).collect::<Vec<_>>().join("; ");
        let row = Row { path: site[0].path.clone(), line: site[0].line, what: format!("HOST({what})"), effect: false };
        classify(&row, &site, &mut faults);
    }
    for t in comments.iter().filter(|t| t.apis.iter().any(|a| CLASSES.contains(&a.as_str()))) {
        if !used.contains(&(t.path.clone(), t.line)) {
            let row = Row { path: t.path.clone(), line: t.line, what: format!("HOST({})", t.apis.join(", ")), effect: false };
            faults.push(("a class above nothing it classes".to_owned(), row));
        }
    }
    faults.sort_by(|a, b| (&a.1.path, a.1.line, &a.0).cmp(&(&b.1.path, b.1.line, &b.0)));
    faults
}

/// The exception comments outside host.rs that name a host API, grouped by
/// the line they cover: one site, however many comments are stacked above it.
fn exception_sites(comments: &[Tag]) -> Vec<Vec<&Tag>> {
    let mut sites: BTreeMap<(&str, usize, usize), Vec<&Tag>> = BTreeMap::new();
    for t in comments {
        let inside = t.path == BOUNDARY || t.path.starts_with(BOUNDARY_MODULES);
        let host = t.apis.iter().any(|a| !CLASSES.contains(&a.as_str()) && !fences_state(a));
        if !inside && host {
            // A comment that covers no line is a site of its own.
            let key = t.covers.map_or((t.path.as_str(), 0, t.line), |c| (t.path.as_str(), c, 0));
            sites.entry(key).or_default().push(t);
        }
    }
    sites.into_values().collect()
}

/// L1-0's closing rule: every host access names exactly one class.
#[test]
fn host_classes_hold() {
    let base = repository();
    let (_, comments, rows) = scan(&base, CRATES.iter().flat_map(|c| roots(&base.join(c))).collect());
    let faults = census(&rows, &comments);
    assert!(faults.is_empty(), "host classes: {} of {} host accesses need one class, `// HOST(CLASS): REASON` above \
                                each host.rs function and descriptor operation, and `// HOST(API, ..., CLASS): REASON` at \
                                an exception site; a class is one of {CLASSES:?}:\n{}",
            faults.len(), rows.len() + comments.iter().filter(|t| t.path != BOUNDARY
                && t.apis.iter().any(|a| !CLASSES.contains(&a.as_str()) && !fences_state(a))).count(),
            faults.iter().map(|(f, r)| format!("  {f}: {}:{} {}", r.path, r.line, r.what)).collect::<Vec<_>>().join("\n"));
}

/// The census of one source file REL: each fault with what it names, no line.
fn census_of(rel: &str, source: &str) -> Vec<(String, String)> {
    let (_, comments, _, rows) = file_scan(rel, source);
    census(&rows, &comments).into_iter().map(|(fault, row)| (fault, row.what)).collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected.iter().map(|&(f, w)| (f.to_owned(), w.to_owned())).collect()
}

#[test]
fn census_lists_what_host_rs_gives_the_workspace() {
    let source = "\
pub fn open(path: &Path) -> io::Result<File> { File::open(path) }
pub(crate) fn wait(child: &Child) -> io::Result<i32> { child.wait() }
fn helper() {}
pub fn outer() {
    fn nested() {}
}
pub struct Input(File);
impl Input {
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.0.read(buf) }
    fn private(&self) {}
}
impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;
    fn next(&mut self) -> Option<Self::Item> { None }
}
impl<'a> Drop for Guard<'a> {
    fn drop(&mut self) {}
}
pub trait Probe {
    fn probe(&self);
}
pub mod call_deadline {
    pub fn arm() {}
    pub(super) fn waiter() {}
    fn run() {}
}
pub unsafe extern \"C\" fn handler(signal: i32) {}
pub fn lookup(buf: [u8; 16]) -> Option<([u8; 16], usize)> { None }
#[cfg(test)]
mod tests {
    pub fn fixture() {}
}
";
    let listed: Vec<String> = census_of(BOUNDARY, source).into_iter().map(|(_, what)| what).collect();
    assert_eq!(listed, [
        "fn open", "fn wait", "fn outer", "fn Input::read", "fn ReadDir::next", "fn Guard::drop",
        "fn call_deadline::arm", "fn handler", "fn lookup",
    ]);
}

#[test]
fn a_class_above_a_host_function_classes_it() {
    let source = "\
// HOST(acquisition): makes the file
pub fn create(path: &Path) -> io::Result<File> {
    effect(|| format!(\"create {}\", bytes(path)))?;
    File::create(path)
}
/// The doc comment and attributes may come between.
// HOST(pure): a regular file's bytes
#[inline]
#[allow(
    clippy::x,
)]
pub fn read_file(path: &Path) -> io::Result<Vec<u8>> { std::fs::read(path) }
// HOST(block): waits for the child
/// Either order.
pub fn wait(child: &Child) -> io::Result<i32> { child.wait() }
impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;
    // HOST(pure): the next entry of a directory host.rs opened
    fn next(&mut self) -> Option<Self::Item> { None }
}
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[]));
}

#[test]
fn an_unclassed_host_write_fails() {
    let source = "\
pub fn remove_file(path: &Path) -> io::Result<()> {
    effect(|| format!(\"remove {}\", bytes(path)))?;
    std::fs::remove_file(path)
}
// Renames FROM to TO: a comment, but no HOST class.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> { std::fs::rename(from, to) }
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[("unclassed", "fn remove_file"), ("unclassed", "fn rename")]));
}

#[test]
fn an_unknown_class_fails() {
    let source = "\
// HOST(escapes): a misspelt class
pub fn rename(from: &Path, to: &Path) -> io::Result<()> { std::fs::rename(from, to) }
// HOST(Escape): classes are lower case
pub fn remove(path: &Path) -> io::Result<()> { std::fs::remove_file(path) }
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[
        ("unknown class `escapes`", "fn rename"),
        ("unknown class `Escape`", "fn remove"),
    ]));
}

#[test]
fn two_classes_on_one_site_fail() {
    let source = "\
// HOST(acquisition, block): spawns, then waits
pub fn call_process(argv: &[CString]) -> io::Result<i32> { spawn(argv)?.wait() }
// HOST(escape): writes
// HOST(pure): and reads
pub fn write(file: &File, bytes: &[u8]) -> io::Result<()> { Ok(()) }
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[
        ("two classes `acquisition`, `block`", "fn call_process"),
        ("two classes `escape`, `pure`", "fn write"),
    ]));
}

#[test]
fn a_pure_host_function_that_performs_an_effect_fails() {
    let source = "\
// HOST(pure): claims to be a query
pub fn chmod(path: &Path, mode: u32) -> io::Result<()> {
    effect(|| format!(\"chmod {}\", bytes(path)))?;
    sys::chmod(path, mode)
}
// HOST(pure): a method named effect is no effect
pub fn describe(fact: &Fact) -> String { fact.effect().to_string() }
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[("pure, but its body calls effect(", "fn chmod")]));
}

#[test]
fn a_class_above_nothing_fails() {
    let source = "\
// HOST(escape): above a private helper, which carries none
fn helper() {}
// HOST(pure): a blank line away

pub fn pid() -> u32 { 0 }
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[
        ("a class above nothing it classes", "HOST(escape)"),
        ("a class above nothing it classes", "HOST(pure)"),
        ("unclassed", "fn pid"),
    ]));
}

#[test]
fn an_exception_site_outside_host_rs_names_its_class() {
    let source = "\
fn store(path: &Path, bytes: &[u8]) -> Result<(), String> {
    // HOST(std::fs): the code-image store
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    // HOST(std::fs, acquisition): the code-image store's new file
    std::fs::write(path.join(\"pages\"), bytes).map_err(|e| e.to_string())?;
    // HOST(std::env::var, pure, block): two classes
    let knob = std::env::var(\"KNOB\").ok();
    // HOST(std::env::var, pur): an unknown class
    let other = std::env::var(\"OTHER\").ok();
    Ok(())
}
fn state(vm: &Vm) {
    // HOST(dynamic:saved): the VM's own binding stack: no host access, no class
    let saved = vm.dynamic.last();
}
";
    assert_eq!(census_of("src/lib.rs", source), pairs(&[
        ("unclassed", "HOST(std::fs)"),
        ("two classes `pure`, `block`", "HOST(std::env::var, pure, block)"),
        ("unknown class `pur`", "HOST(std::env::var, pur)"),
    ]));
}

#[test]
fn a_descriptor_operation_outside_host_rs_names_its_class() {
    let source = "\
fn send(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    rustix::net::send(fd, buf, rustix::net::SendFlags::NOSIGNAL)
}
fn wait(set: &mut [libc::pollfd]) -> i32 {
    // HOST(block): waits for a process's output
    unsafe { libc::poll(set.as_mut_ptr(), set.len() as libc::nfds_t, -1) }
}
fn print(bytes: &[u8]) {
    // HOST(escape): stdout is a send
    std::io::stdout().write_all(bytes).unwrap();
    eprintln!(\"done\");
    let text = \"println!(not code)\";
}
fn save(file: &mut File, bytes: &[u8]) -> io::Result<()> {
    // HOST(OpenOptions, std::fs, acquisition): one comment classes the API and the write
    std::fs::OpenOptions::new().append(true).open(\"log\")?.write_all(bytes)
}
#[cfg(test)]
mod tests {
    fn t() { println!(\"test output\"); }
}
";
    assert_eq!(census_of("src/lib.rs", source), pairs(&[
        ("unclassed", "descriptor:write `rustix::net::send(fd, buf, rustix::net::SendFlags::NOSIGNAL)`"),
        ("unclassed", "stdio `eprintln!(\"done\");`"),
    ]));
}

#[test]
fn stacked_classes_over_one_exception_site_fail() {
    let source = "\
fn create(path: &Path) -> std::io::Result<File> {
    // HOST(std::fs, acquisition): makes the file
    // HOST(OpenOptions, block): and claims to wait
    std::fs::OpenOptions::new().create_new(true).write(true).open(path)
}
fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    // HOST(std::fs, block): a read by path
    // HOST(std::env::var): one class covers the site
    std::fs::read(std::env::var(\"P\").unwrap_or_default())
}
";
    assert_eq!(census_of("src/lib.rs", source), pairs(&[
        ("two classes `acquisition`, `block`", "HOST(std::fs, acquisition; OpenOptions, block)"),
    ]));
}

#[test]
fn a_qualified_effect_call_counts_and_a_nested_function_does_not() {
    let source = "\
// HOST(pure): claims to be a query
pub fn chmod(path: &Path, mode: u32) -> io::Result<()> {
    self::effect(|| format!(\"chmod {}\", bytes(path)))?;
    sys::chmod(path, mode)
}
// HOST(pure): claims to be a query
pub fn remove(path: &Path) -> io::Result<()> {
    crate::host::effect(|| format!(\"remove {}\", bytes(path)))?;
    sys::remove(path)
}
// HOST(pure): another type's effect is no gate
pub fn describe(fact: &Fact) -> String { Fact::effect(fact).to_string() }
// HOST(pure): a nested helper's gate is not this body's
pub fn pid() -> u32 {
    fn helper() -> io::Result<()> { effect(|| String::new()) }
    0
}
";
    assert_eq!(census_of(BOUNDARY, source), pairs(&[
        ("pure, but its body calls effect(", "fn chmod"),
        ("pure, but its body calls effect(", "fn remove"),
    ]));
}

#[test]
fn a_process_operation_outside_host_rs_names_its_class() {
    let source = "\
fn run(mut command: Command) -> io::Result<()> {
    let mut child = command.spawn()?;
    let _ = command.output()?;
    let _ = command.status()?;
    let _ = child.try_wait()?;
    let _ = child.wait()?;
    let db = Connection::open(name, false, true, true);
    // HOST(acquisition): starts the child its guard let through
    let other = command.spawn()?;
    Ok(())
}
";
    let found = census_of("src/lib.rs", source);
    assert_eq!(found, pairs(&[
        ("unclassed", "process `let mut child = command.spawn()?;`"),
        ("unclassed", "process `let _ = command.output()?;`"),
        ("unclassed", "process `let _ = command.status()?;`"),
        ("unclassed", "process `let _ = child.try_wait()?;`"),
        ("unclassed", "process `let _ = child.wait()?;`"),
        ("unclassed", "process `let db = Connection::open(name, false, true, true);`"),
    ]));
}

#[test]
fn a_descriptor_mutation_duplicate_or_query_names_its_class() {
    let source = "\
fn file(output: &File, times: FileTimes, fd: BorrowedFd<'_>) -> io::Result<()> {
    output.set_times(times)?;
    rustix::fs::fallocate(fd, FallocateFlags::empty(), 0, 4096)?;
    let copy = output.try_clone()?;
    let error = rustix::net::sockopt::socket_error(fd);
    let modes = rustix::termios::tcgetattr(fd);
    // HOST(escape): the child's own session, before exec
    unsafe { libc::setsid() };
    Ok(())
}
";
    assert_eq!(census_of("src/lib.rs", source), pairs(&[
        ("unclassed", "descriptor:write `output.set_times(times)?;`"),
        ("unclassed", "descriptor:write `rustix::fs::fallocate(fd, FallocateFlags::empty(), 0, 4096)?;`"),
        ("unclassed", "descriptor:dup `let copy = output.try_clone()?;`"),
        ("unclassed", "descriptor:read `let error = rustix::net::sockopt::socket_error(fd);`"),
        ("unclassed", "descriptor:query `let modes = rustix::termios::tcgetattr(fd);`"),
    ]));
}
