//! fns.c `Flocale_info`'s `nl_langinfo` and sysdep.c `str_collate`, for the
//! locale the environment names, which GNU's startup selects with setlocale
//! (`LC_ALL`, ""): read through a locale object, so the process's own C
//! locale stays as Rust left it.
//!
//! `Locale` is the only unsafe code here. glibc's locale objects
//! (newlocale, `nl_langinfo_l`, `towlower_l`, `wcscoll_l`) have no safe crate:
//! icu's collation is not glibc's, and `string-collate-lessp` answers in
//! glibc's order.

/// What `locale-info' asks for.
pub(crate) enum Item { Codeset, Days, Months, Paper }

/// What it answers: bytes in the locale's encoding, or the paper's width
/// and height in millimeters.
pub(crate) enum Info { Text(Vec<u8>), Texts(Vec<Vec<u8>>), Paper(i64, i64) }

pub(crate) fn query(item: Item) -> Option<Info> {
    // setlocale keeps "C" for a locale the system does not have.
    let locale = Locale::new(libc::LC_ALL_MASK, c"").or_else(|_| Locale::new(libc::LC_ALL_MASK, c"C")).ok()?;
    let text = |item: libc::nl_item| locale.text(item);
    match item {
        Item::Codeset => Some(Info::Text(text(libc::CODESET))),
        Item::Days => Some(Info::Texts([libc::DAY_1, libc::DAY_2, libc::DAY_3, libc::DAY_4, libc::DAY_5, libc::DAY_6, libc::DAY_7]
            .into_iter().map(text).collect())),
        Item::Months => Some(Info::Texts([libc::MON_1, libc::MON_2, libc::MON_3, libc::MON_4, libc::MON_5, libc::MON_6,
                                          libc::MON_7, libc::MON_8, libc::MON_9, libc::MON_10, libc::MON_11, libc::MON_12]
            .into_iter().map(text).collect())),
        // glibc's _NL_PAPER_HEIGHT and _NL_PAPER_WIDTH, _NL_ITEM (LC_PAPER, 0
        // and 1), are integers in the pointer nl_langinfo returns.
        Item::Paper if cfg!(target_env = "gnu") => Some(Info::Paper(locale.number(0x70001), locale.number(0x70000))),
        Item::Paper => None,
    }
}

/// sysdep.c str_collate: wcscoll of character codes A and B, each read up to
/// its first NUL as a wide string is, in LOCALE's collation (a locale name),
/// else the environment's, as GNU's startup setlocale (LC_ALL, "") selects;
/// IGNORE_CASE lowers both first (towlower). The error is GNU's message.
pub(crate) fn collate(a: &[u32], b: &[u32], locale: Option<&std::ffi::CStr>, ignore_case: bool) -> Result<std::cmp::Ordering, String> {
    let mask = libc::LC_COLLATE_MASK | libc::LC_CTYPE_MASK;
    let name = locale.unwrap_or(c"");
    let loc = match Locale::new(mask, name) {
        Err(_) if locale.is_none() => Locale::new(mask, c"C"),
        found => found,
    };
    let loc = loc.map_err(|err| {
        format!("Invalid locale {}: {}", name.to_string_lossy(), crate::prim::strerror(err.raw_os_error().unwrap_or(0)))
    })?;
    let wide = |s: &[u32]| -> Vec<libc::wchar_t> {
        s.iter().map(|&c| if ignore_case { loc.lower(c) } else { c }.cast_signed()).chain([0]).collect()
    };
    loc.collate(&wide(a), &wide(b))
        .map(|res| res.cmp(&0))
        .map_err(|err| format!("Invalid string for collation: {}", crate::prim::strerror(err)))
}

// UNSAFE(the libc crate does not declare glibc's wcscoll_l or towlower_l)
// SAFETY: these are glibc's own prototypes (wchar.h, wctype.h): wcscoll_l
// takes two NUL-terminated wide strings and a locale, towlower_l a wide
// character and a locale.
unsafe extern "C" {
    fn wcscoll_l(a: *const libc::wchar_t, b: *const libc::wchar_t, locale: libc::locale_t) -> libc::c_int;
    fn towlower_l(c: u32, locale: libc::locale_t) -> u32;
}

/// A glibc locale object, never null, freed when dropped.
struct Locale(libc::locale_t);

impl Locale {
    /// newlocale(MASK, NAME): the categories of MASK from the locale NAME.
    fn new(mask: libc::c_int, name: &std::ffi::CStr) -> std::io::Result<Self> {
        // UNSAFE(no crate makes glibc locale objects: newlocale is a C call)
        // SAFETY: NAME is a NUL-terminated string the call only reads; a null
        // base asks for a new object, which this value owns from here.
        let made = unsafe { libc::newlocale(mask, name.as_ptr(), std::ptr::null_mut()) };
        if made.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(Self(made)) }
    }

    /// `nl_langinfo_l`'s text for ITEM, copied out.
    fn text(&self, item: libc::nl_item) -> Vec<u8> {
        // UNSAFE(no crate reads glibc's nl_langinfo_l: it returns a C string)
        // SAFETY: for a text item nl_langinfo_l returns a NUL-terminated
        // string, never null, valid while the locale lives; it is copied
        // before `self` can drop.
        unsafe { std::ffi::CStr::from_ptr(self.langinfo(item)) }.to_bytes().to_vec()
    }

    /// `nl_langinfo_l`'s integer for ITEM: glibc keeps some items' values
    /// in the low bits of the pointer it returns.
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "glibc keeps an int in the pointer's low 32 bits")]
    fn number(&self, item: libc::nl_item) -> i64 {
        i64::from(self.langinfo(item).addr() as i32)
    }

    fn langinfo(&self, item: libc::nl_item) -> *const libc::c_char {
        // UNSAFE(no crate exposes glibc's nl_langinfo_l: a C call)
        // SAFETY: the call takes a number and reads only the live locale
        // object; what it returns is read only by `text`, for text items.
        unsafe { libc::nl_langinfo_l(item, self.0) }
    }

    /// `towlower_l` of C.
    fn lower(&self, c: u32) -> u32 {
        // UNSAFE(no crate exposes glibc's towlower_l: a C call)
        // SAFETY: towlower_l takes a number and reads only the live locale.
        unsafe { towlower_l(c, self.0) }
    }

    /// `wcscoll_l` of the NUL-terminated wide strings A and B, or the errno
    /// it set.
    fn collate(&self, a: &[libc::wchar_t], b: &[libc::wchar_t]) -> Result<libc::c_int, i32> {
        assert!(a.last() == Some(&0) && b.last() == Some(&0), "wcscoll_l reads up to a NUL");
        errno::set_errno(errno::Errno(0));
        // UNSAFE(no crate exposes glibc's wcscoll_l: a C call)
        // SAFETY: A and B end in a NUL, so wcscoll_l reads only within them,
        // and the locale object is live.
        let res = unsafe { wcscoll_l(a.as_ptr(), b.as_ptr(), self.0) };
        match errno::errno().0 {
            0 => Ok(res),
            err => Err(err),
        }
    }
}

impl Drop for Locale {
    fn drop(&mut self) {
        // UNSAFE(no crate frees glibc locale objects: freelocale is a C call)
        // SAFETY: the object came from newlocale, is owned by this value
        // alone, and is freed once, here.
        unsafe { libc::freelocale(self.0) };
    }
}
