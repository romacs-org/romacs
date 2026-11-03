//! Native GNU interlocking protocol (src/filelock.c), with traced T2 callbacks.
//! See evidence/filelock-save/REPORT.md for platform/representation boundaries.
use super::*;
use crate::forwarded::VisitedModtime;
use std::io::Write;
use crate::host;
use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
use std::path::Path;

fn variable(cx: &mut Leaf<'_>, name: &str) -> Out {
    let symbol = cx.intern(name);
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL))
}
fn nil_done(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next { Ok(Step::Done(Value::NIL)) }
fn call(cx: &mut Leaf<'_>, name: &str, args: Vec<Value>, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    Ok(Step::Call { f: cx.intern(name), args, then, saved })
}
fn io_fault(cx: &mut Leaf<'_>, operation: &str, file: Value, e: std::io::Error) -> Fault {
    let message = e.to_string();
    let message = message.rsplit_once(" (os error ").map_or(message.as_str(), |(s, _)| s);
    let tail = cx.cons(file, Value::NIL);
    let message = cx.string(message);
    let tail = cx.cons(message, tail);
    let operation = cx.string(operation);
    let data = cx.cons(operation, tail);
    let kind = match e.raw_os_error() { Some(libc::ENOENT) => "file-missing", Some(libc::EACCES) => "permission-denied", _ => "file-error" };
    Fault::Signal(cx.intern(kind), data)
}
fn errno(n: i32) -> std::io::Error { std::io::Error::from_raw_os_error(n) }
fn host(cx: &mut Leaf<'_>) -> std::result::Result<Vec<u8>, Fault> {
    let name = variable(cx, "system-name")?;
    if name == Value::NIL { return Ok(Vec::new()); }
    Ok(cx.string_bytes(name)?.into_iter().map(|c| if c == b'@' { b'-' } else { c }).collect())
}
fn login(cx: &mut Leaf<'_>) -> std::result::Result<Vec<u8>, Fault> {
    let name = variable(cx, "user-login-name")?;
    if name != Value::NIL { return Ok(cx.string_bytes(name).unwrap_or_default()); }
    if let Some(name) = host::getenv("LOGNAME").or_else(|| host::getenv("USER")) {
        return Ok(name.as_os_str().as_bytes().to_vec());
    }
    Ok(cx.filelock_login_name())
}
fn identity(cx: &mut Leaf<'_>) -> std::result::Result<Vec<u8>, Fault> {
    let mut id = login(cx)?; id.push(b'@'); id.extend(host(cx)?);
    id.extend(format!(".{}", host::pid()).bytes());
    // GNU accepts legacy identities without a boot field. Do not publish a
    // /proc/stat timestamp: it differs from GNU gnulib boot-time on this host.
    Ok(id)
}
#[derive(Debug)]
enum Owner { Free, Ours, Other { user: Vec<u8>, diagnostic: Vec<u8> } }
fn read_lock(path: &Path) -> std::io::Result<Vec<u8>> {
    loop {
        match host::read_link(path) {
            Ok(target) => return Ok(target.as_os_str().as_bytes().to_vec()),
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
                match host::open_input(path, libc::O_NOFOLLOW | libc::O_NONBLOCK) {
                    Ok(mut file) => {
                        // GNU can block on a FIFO. Refuse nonregular lock files.
                        if !file.stat()?.file_type().is_file() { return Err(errno(libc::EINVAL)); }
                        let read = file.read_range(0, 8193);
                        file.close()?;
                        return read;
                    }
                    Err(e) if e.raw_os_error() == Some(libc::ELOOP) => continue,
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
}
fn decimal(bytes: &[u8]) -> Option<(i64, usize)> {
    let start = usize::from(bytes.first() == Some(&b'-'));
    let end = start + bytes[start..].iter().take_while(|b| b.is_ascii_digit()).count();
    if end == start { return None; }
    Some((std::str::from_utf8(&bytes[..end]).ok()?.parse().unwrap_or(if start == 1 { i64::MIN } else { i64::MAX }), end))
}
fn owner(cx: &Leaf<'_>, path: &Path, local_host: &[u8]) -> std::io::Result<Owner> {
    let bytes = match read_lock(path) {
        Ok(bytes) => bytes,
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) => return Ok(Owner::Free),
        Err(e) => return Err(e),
    };
    if bytes.len() > 8192 { return Err(errno(libc::ENAMETOOLONG)); }
    if !bytes.is_empty() {
        let at = bytes.iter().rposition(|b| *b == b'@').ok_or_else(|| errno(libc::EINVAL))?;
        let dot = bytes[at..].iter().rposition(|b| *b == b'.').map(|i| at + i).ok_or_else(|| errno(libc::EINVAL))?;
        let (pid, count) = decimal(&bytes[dot+1..]).ok_or_else(|| errno(libc::EINVAL))?;
        let rest = &bytes[dot+1+count..];
        let _boot = if rest.is_empty() { 0 } else {
            let text = rest.strip_prefix(b":").or_else(|| rest.strip_prefix(b"\xef\x80\xa2")).ok_or_else(|| errno(libc::EINVAL))?;
            let (boot, n) = decimal(text).ok_or_else(|| errno(libc::EINVAL))?;
            if n != text.len() { return Err(errno(libc::EINVAL)); } boot
        };
        let local = &bytes[at+1..dot] == local_host;
        if local && pid == i64::from(host::pid()) { return Ok(Owner::Ours); }
        let alive = if local && pid > 0 && pid <= i64::from(i32::MAX) {
            // kill(pid, 0) probes existence/permission only; sends no signal.
            cx.filelock_process_exists(pid as i32)
        } else { false };
        // Conservatively retain any live local process lock. Reboot/PID reuse
        // disambiguation is withheld until GNU-compatible boot identity exists.
        if !local || alive {
            let mut diagnostic = bytes[..dot].to_vec(); diagnostic.extend(b" (pid ");
            diagnostic.extend(&bytes[dot+1..dot+1+count]); diagnostic.push(b')');
            return Ok(Owner::Other { user: bytes[..at].to_vec(), diagnostic });
        }
    }
    match host::remove_file(path) { Ok(()) => Ok(Owner::Free), Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(Owner::Free), Err(e) => Err(e) }
}
fn create(cx: &Leaf<'_>, path: &Path, identity: &[u8], force: bool) -> std::io::Result<()> {
    if identity.len() > 8192 { return Err(errno(libc::ENAMETOOLONG)); }
    let target = std::ffi::OsStr::from_bytes(identity);
    let mut result = host::symlink(target, path);
    if force && result.as_ref().is_err_and(|e| e.raw_os_error() == Some(libc::EEXIST)) {
        let _ = host::remove_file(path); result = host::symlink(target, path);
    }
    if !result.as_ref().is_err_and(|e| matches!(e.raw_os_error(), Some(libc::ENOSYS | libc::EPERM | libc::ENAMETOOLONG))) { return result; }
    let (nonce, mut file) = cx.filelock_create_nonce(path)?;
    let result = (|| {
        file.write_all(identity)?; file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        cx.fileio_close_output(file)?;
        if force { host::rename(&nonce, path) } else { host::hard_link(&nonce, path) }
    })();
    let _ = host::remove_file(nonce); result
}

// MODE: query 0, lock 1, unlock 2; HANDLED returns from FILE's handler.
// All Lisp values crossing callbacks are saved.
fn dispatch(cx: &mut Leaf<'_>, file: Value, mode: i64, handled: crate::prim::Then) -> Next {
    let operation = cx.intern(match mode { 0 => "file-locked-p", 1 => "lock-file", _ => "unlock-file" });
    let handler = super::fileio::find_file_name_handler(cx, file, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, file], then: handled, saved: vec![] });
    }
    if mode == 1 && variable(cx, "create-lockfiles")? == Value::NIL {
        return supersession(cx, Value::NIL, &[file, Value::NIL]);
    }
    // make_lock_file_name: Fexpand_file_name, then the Lisp naming.
    super::fileio::expanding(cx, file, lock_name_expanded, vec![file, Value::integer(mode).unwrap()])
}
fn lock_name_expanded(cx: &mut Leaf<'_>, name: Value, saved: &[Value]) -> Next {
    // Bootstrap regime: before files.el defines the naming, a file has no
    // lock name, as when make-lock-file-name returns nil.
    let naming = cx.intern("make-lock-file-name");
    if !cx.fboundp(naming) { return got_lock_name(cx, Value::NIL, saved); }
    call(cx, "make-lock-file-name", vec![name], got_lock_name, saved.to_vec())
}
fn query_done(_: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next { Ok(Step::Done(result)) }
fn got_lock_name(cx: &mut Leaf<'_>, lock: Value, saved: &[Value]) -> Next {
    let mode = saved[1].integer_value().unwrap();
    if lock == Value::NIL { return Ok(Step::Done(Value::NIL)); }
    let path = super::fileio::file_name_path(cx, lock)?;
    if path.as_os_str().as_encoded_bytes().contains(&0) { return Err(Fault::Wrong("filenamep", lock)); }
    let local_host = host(cx)?;
    if mode == 0 {
        return match owner(cx, &path, &local_host) {
            Ok(Owner::Free) => Ok(Step::Done(Value::NIL)), Ok(Owner::Ours) => Ok(Step::Done(Value::TRUE)),
            Ok(Owner::Other { user, .. }) => Ok(Step::Done(cx.make_string(user, false))),
            Err(e) => Err(io_fault(cx, "Testing file lock", saved[0], e)),
        };
    }
    if mode == 2 {
        let result = owner(cx, &path, &local_host).and_then(|o| if matches!(o, Owner::Ours) { host::remove_file(&path) } else { Ok(()) });
        if let Err(e) = result {
            if e.raw_os_error() != Some(libc::ENOENT) {
                let Fault::Signal(condition, data) = io_fault(cx, "Unlocking file", saved[0], e) else { unreachable!() };
                let error = cx.cons(condition, data);
                return call(cx, "userlock--handle-unlock-error", vec![error], nil_done, vec![]);
            }
        }
        return Ok(Step::Done(Value::NIL));
    }
    supersession(cx, lock, &[saved[0], lock])
}
fn supersession(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let name = super::fileio::file_name_path(cx, saved[0])?;
    let state = match cx.visit_with_truename(&name) {
        Some(buffer) => Some(cx.fileio_modtime(buffer)?.0),
        None => None,
    };
    let threatened = state.is_some_and(|state| match state {
        VisitedModtime::Unknown => false,
        VisitedModtime::Missing => host::metadata(&name).is_ok(),
        VisitedModtime::Known { seconds, nanos, size } => {
            use std::os::unix::fs::MetadataExt;
            host::metadata(&name).is_ok_and(|m| m.mtime() != seconds || m.mtime_nsec() != nanos || size.is_some_and(|s| s != m.len()))
        }
    });
    let ours = if saved[1] == Value::NIL { false } else {
        let path = super::fileio::file_name_path(cx, saved[1])?;
        let local_host = host(cx)?;
        matches!(owner(cx, &path, &local_host), Ok(Owner::Ours))
    };
    if threatened && !ours {
        return call(cx, "userlock--ask-user-about-supersession-threat", vec![saved[0]], acquire, saved.to_vec());
    }
    acquire(cx, Value::NIL, saved)
}
fn acquire(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    if saved[1] == Value::NIL { return Ok(Step::Done(Value::NIL)); }
    let path = super::fileio::file_name_path(cx, saved[1])?;
    let identity = identity(cx)?; let local_host = host(cx)?;
    loop {
        match create(cx, &path, &identity, false) {
            Ok(()) => return Ok(Step::Done(Value::NIL)),
            Err(e) if e.raw_os_error() == Some(libc::EEXIST) => match owner(cx, &path, &local_host) {
                Ok(Owner::Free) => continue, Ok(Owner::Ours) => return Ok(Step::Done(Value::NIL)),
                Ok(Owner::Other { diagnostic, .. }) => {
                    let who = cx.make_string(diagnostic, false);
                    return call(cx, "ask-user-about-lock", vec![saved[0], who], attack, vec![saved[1]]);
                }
                Err(_) => return Ok(Step::Done(Value::NIL)),
            },
            Err(_) => return Ok(Step::Done(Value::NIL)),
        }
    }
}
fn attack(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    if result != Value::NIL {
        let path = super::fileio::file_name_path(cx, saved[0])?; let identity = identity(cx)?;
        let _ = create(cx, &path, &identity, true);
    }
    Ok(Step::Done(Value::NIL))
}

fn current_truename(cx: &mut Leaf<'_>) -> Out {
    let current = cx.current_buffer()?;
    Ok(cx.visit_truename(current).map_or(Value::NIL, |name| match std::str::from_utf8(&name) {
            // The truename a buffer records is a string, as bset_current recorded it.
            Ok(text) => cx.string(text),
            Err(_) => super::fileio::file_name_value(cx, &name),
        }))
}

defsubr! { name = "lock-buffer", gnu = "filelock.c:725", effect = Global,
fn lock_buffer(cx; optional file: Value) -> Step {
    let file = if file == Value::NIL { current_truename(cx)? } else { cx.string_view(file)?; file };
    let state = cx.buffer_modification(Value::NIL)?;
    if file == Value::NIL || state.modiff <= state.save_modiff { return Ok(Step::Done(Value::NIL)); }
    dispatch(cx, file, 1, nil_done)
}}
defsubr! { name = "unlock-buffer", gnu = "filelock.c:744", effect = Global,
fn unlock_buffer(cx) -> Step {
    let state = cx.buffer_modification(Value::NIL)?;
    if state.modiff <= state.save_modiff { return Ok(Step::Done(Value::NIL)); }
    let file = current_truename(cx)?;
    if file == Value::NIL { return Ok(Step::Done(Value::NIL)); }
    dispatch(cx, file, 2, nil_done)
}}
defsubr! { name = "lock-file", gnu = "filelock.c:677", effect = Global,
fn lock_file(cx, file: Value) -> Step {
    cx.string_view(file)?;
    dispatch(cx, file, 1, query_done)
}}
defsubr! { name = "unlock-file", gnu = "filelock.c:700", effect = Global,
fn unlock_file(cx, file: Value) -> Step {
    cx.string_view(file)?;
    dispatch(cx, file, 2, nil_done)
}}
defsubr! { name = "file-locked-p", gnu = "filelock.c:770", effect = Global,
fn file_locked_p(cx, filename: Value) -> Step { dispatch(cx, filename, 0, query_done) }}
