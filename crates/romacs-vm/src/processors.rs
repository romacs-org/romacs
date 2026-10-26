//! gnulib nproc's processor counts, through the C library.

/// NPROC_CURRENT: the processors this process may run on.
pub(crate) fn current() -> u64 {
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set) } == 0 {
        let count = unsafe { libc::CPU_COUNT(&set) };
        if count > 0 { return count as u64; }
    }
    configured(false)
}

/// NPROC_ALL with ALL, else the online processors.
pub(crate) fn configured(all: bool) -> u64 {
    let name = if all { libc::_SC_NPROCESSORS_CONF } else { libc::_SC_NPROCESSORS_ONLN };
    u64::try_from(unsafe { libc::sysconf(name) }).ok().filter(|&n| n > 0).unwrap_or(1)
}
