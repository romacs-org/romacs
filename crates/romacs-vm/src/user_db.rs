//! The user database as GNU editfns.c sees it: getuid/geteuid and
//! getpwuid/getpwnam through the C library (so NSS sources count).
pub(crate) struct Account {
    pub name: String,
    pub gecos: String,
}

pub(crate) fn real_uid() -> u32 {
    unsafe { libc::getuid() }
}

pub(crate) fn effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn account(lookup: impl Fn(*mut libc::passwd, *mut libc::c_char, usize, *mut *mut libc::passwd) -> libc::c_int) -> Option<Account> {
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    if lookup(&mut entry, buffer.as_mut_ptr(), buffer.len(), &mut found) != 0 || found.is_null() {
        return None;
    }
    let text = |p: *const libc::c_char| if p.is_null() { String::new() } else {
        unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
    };
    Some(Account { name: text(entry.pw_name), gecos: text(entry.pw_gecos) })
}

pub(crate) fn by_uid(uid: u32) -> Option<Account> {
    account(|entry, buffer, size, found| unsafe { libc::getpwuid_r(uid, entry, buffer, size, found) })
}

pub(crate) fn by_name(name: &str) -> Option<Account> {
    let name = std::ffi::CString::new(name).ok()?;
    account(|entry, buffer, size, found| unsafe { libc::getpwnam_r(name.as_ptr(), entry, buffer, size, found) })
}
