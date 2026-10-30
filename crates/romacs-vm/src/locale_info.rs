//! fns.c Flocale_info's nl_langinfo, for the locale the environment names,
//! which GNU's startup selects with setlocale (LC_ALL, ""): read through a
//! locale object, so the process's own C locale stays as Rust left it.

/// What `locale-info' asks for.
pub(crate) enum Item { Codeset, Days, Months, Paper }

/// What it answers: bytes in the locale's encoding, or the paper's width
/// and height in millimeters.
pub(crate) enum Info { Text(Vec<u8>), Texts(Vec<Vec<u8>>), Paper(i64, i64) }

pub(crate) fn query(item: Item) -> Option<Info> {
    // setlocale keeps "C" for a locale the system does not have.
    let locale = unsafe {
        let named = libc::newlocale(libc::LC_ALL_MASK, c"".as_ptr(), std::ptr::null_mut());
        if named.is_null() { libc::newlocale(libc::LC_ALL_MASK, c"C".as_ptr(), std::ptr::null_mut()) } else { named }
    };
    if locale.is_null() { return None; }
    let text = |item: libc::nl_item| unsafe { std::ffi::CStr::from_ptr(libc::nl_langinfo_l(item, locale)).to_bytes().to_vec() };
    let info = match item {
        Item::Codeset => Some(Info::Text(text(libc::CODESET))),
        Item::Days => Some(Info::Texts([libc::DAY_1, libc::DAY_2, libc::DAY_3, libc::DAY_4, libc::DAY_5, libc::DAY_6, libc::DAY_7]
            .into_iter().map(text).collect())),
        Item::Months => Some(Info::Texts([libc::MON_1, libc::MON_2, libc::MON_3, libc::MON_4, libc::MON_5, libc::MON_6,
                                          libc::MON_7, libc::MON_8, libc::MON_9, libc::MON_10, libc::MON_11, libc::MON_12]
            .into_iter().map(text).collect())),
        // glibc's _NL_PAPER_HEIGHT and _NL_PAPER_WIDTH, _NL_ITEM (LC_PAPER, 0
        // and 1), are integers in the pointer nl_langinfo returns.
        Item::Paper if cfg!(target_env = "gnu") => {
            let number = |item: libc::nl_item| unsafe { libc::nl_langinfo_l(item, locale) as isize as i32 as i64 };
            Some(Info::Paper(number(0x70001), number(0x70000)))
        }
        Item::Paper => None,
    };
    unsafe { libc::freelocale(locale) };
    info
}
