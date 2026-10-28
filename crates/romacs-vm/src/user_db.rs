//! The user database as GNU editfns.c sees it: getuid/geteuid and
//! getpwuid/getpwnam through the C library (so NSS sources count).
pub(crate) struct Account {
    pub name: String,
    pub gecos: String,
    /// pw_dir, absent when the entry has none.
    pub home: Option<String>,
}

pub(crate) fn real_uid() -> u32 {
    crate::host::uid()
}

pub(crate) fn effective_uid() -> u32 {
    crate::host::euid()
}

const BUFFER: usize = 16 * 1024;

fn account(entry: crate::host::Passwd) -> Account {
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    Account { name: entry.name.map(text).unwrap_or_default(), gecos: text(entry.gecos), home: entry.dir.map(text) }
}

pub(crate) fn by_uid(uid: u32) -> Option<Account> {
    crate::host::user_by_uid(uid, BUFFER).ok().map(account)
}

pub(crate) fn by_name(name: &str) -> Option<Account> {
    let name = std::ffi::CString::new(name).ok()?;
    crate::host::user_by_name(&name, BUFFER).ok().map(account)
}
