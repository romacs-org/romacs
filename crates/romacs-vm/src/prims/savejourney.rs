//! Native GNU fileio.c primitives reached by the real file-visiting journey.
use super::*;

defsubr! { name = "get-truename-buffer", gnu = "buffer.c:524", effect = Global,
fn get_truename_buffer(cx, filename: Value) {
    let field = cx.intern("buffer-file-truename");
    let mut buffers = cx.lifecycle_list();
    while let Some((buffer, rest)) = cx.pair(buffers) {
        buffers = rest;
        if !cx.lifecycle_live_p(buffer) { continue; }
        let truename = cx.buffer_local_read(field, buffer)?;
        let Ok(actual) = cx.string_view(truename) else { continue; };
        let (bytes, chars) = if let Ok(name) = cx.symbol_name(filename) {
            let chars = name.chars().count();
            (name.into_bytes(), chars)
        } else {
            let view = cx.string_view(filename)?;
            (view.bytes, view.chars)
        };
        if actual.bytes == bytes && actual.chars == chars { return Ok(buffer); }
    }
    Ok(Value::NIL)
}}


fn attribute_time(cx: &mut Leaf<'_>, seconds: i64, nanos: i64) -> std::result::Result<Value, Fault> {
    let variable = cx.intern("current-time-list");
    if cx.variable_value(variable)?.unwrap_or(Value::NIL) == Value::NIL {
        let ticks = cx.bigint((i128::from(seconds) * 1_000_000_000 + i128::from(nanos)).into());
        return Ok(cx.cons(ticks, Value::integer(1_000_000_000).unwrap()));
    }
    let mut list = Value::NIL;
    for field in [seconds.div_euclid(65536), seconds.rem_euclid(65536), nanos / 1000, nanos % 1000 * 1000].into_iter().rev() {
        let field = cx.bigint(field.into());
        list = cx.cons(field, list);
    }
    Ok(list)
}

fn file_mode_string(mode: u32) -> Vec<u8> {
    let mut out = b"----------".to_vec();
    out[0] = match mode & libc::S_IFMT { libc::S_IFDIR => b'd', libc::S_IFLNK => b'l', libc::S_IFCHR => b'c', libc::S_IFBLK => b'b', libc::S_IFIFO => b'p', libc::S_IFSOCK => b's', _ => b'-' };
    for (i, (mask, letter)) in [(0o400,b'r'),(0o200,b'w'),(0o100,b'x'),(0o040,b'r'),(0o020,b'w'),(0o010,b'x'),(0o004,b'r'),(0o002,b'w'),(0o001,b'x')].into_iter().enumerate() {
        if mode & mask != 0 { out[i + 1] = letter; }
    }
    for (index, mask, lower, upper) in [(3,0o4000,b's',b'S'),(6,0o2000,b's',b'S'),(9,0o1000,b't',b'T')] {
        if mode & mask != 0 { out[index] = if out[index] == b'x' { lower } else { upper }; }
    }
    out
}

defsubr! { name = "file-attributes", gnu = "dired.c:953", effect = Global,
fn file_attributes(cx, filename: Value; optional id_format: Value) -> Step {
    use std::os::unix::fs::MetadataExt;
    // GNU catches all expansion conditions and returns nil.
    let path = match super::fileio::expand_file_name(cx, filename) {
        Ok(path) => path,
        Err(_) => return Ok(Step::Done(Value::NIL)),
    };
    let expanded = cx.string(&path);
    let operation = cx.intern("file-attributes");
    let handler = super::fileio::find_file_name_handler(cx, expanded, operation)?;
    if handler != Value::NIL {
        let mut args = vec![operation, expanded];
        if id_format != Value::NIL { args.push(id_format); }
        return Ok(Step::Call { f: handler, args, then: returned, saved: vec![] });
    }
    let integer = cx.intern("integer");
    let named = id_format != Value::NIL && id_format != integer;
    let metadata = match crate::host::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(Step::Done(Value::NIL)),
    };
    let file_type = if metadata.file_type().is_dir() { Value::TRUE }
    else if metadata.file_type().is_symlink() {
        let target = match crate::host::read_link(&path) { Ok(target) => target, Err(_) => return Ok(Step::Done(Value::NIL)) };
        let target = target.to_str().ok_or_else(|| Fault::Error("VM2 file-attributes non-UTF-8 symlink target decoding unsupported".into()))?;
        cx.make_string(target.as_bytes().to_vec(), !target.is_ascii())
    } else { Value::NIL };
    let links = cx.bigint(metadata.nlink().into());
    let uid = cx.file_attribute_identity(metadata.uid(), false, named);
    let gid = cx.file_attribute_identity(metadata.gid(), true, named);
    let atime = attribute_time(cx, metadata.atime(), metadata.atime_nsec())?;
    let mtime = attribute_time(cx, metadata.mtime(), metadata.mtime_nsec())?;
    let ctime = attribute_time(cx, metadata.ctime(), metadata.ctime_nsec())?;
    let size = cx.bigint(metadata.size().into());
    let mode = cx.make_string(file_mode_string(metadata.mode()), false);
    let inode = cx.bigint(metadata.ino().into());
    let device = cx.bigint(metadata.dev().into());
    let mut list = Value::NIL;
    for value in [file_type, links, uid, gid, atime, mtime, ctime, size, mode, Value::TRUE, inode, device].into_iter().rev() { list = cx.cons(value, list); }
    Ok(Step::Done(list))
}}

fn symlink_expanded(cx: &mut Leaf<'_>, filename: Value, _: &[Value]) -> Next {
    cx.string_bytes(filename)?;
    let operation = cx.intern("file-symlink-p");
    let handler = super::fileio::find_file_name_handler(cx, filename, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, filename], then: returned, saved: vec![] });
    }
    let path = cx.string_text(filename)?;
    let target = match crate::host::read_link(path) {
        Ok(target) => target,
        Err(_) => return Ok(Step::Done(Value::NIL)),
    };
    let target = target.to_str().ok_or_else(|| Fault::Error("VM2 file-symlink-p non-UTF-8 target decoding unsupported".into()))?;
    Ok(Step::Done(cx.make_string(target.as_bytes().to_vec(), !target.is_ascii())))
}

defsubr! { name = "file-symlink-p", gnu = "fileio.c:3162", effect = Global,
fn file_symlink_p(cx, filename: Value) -> Step {
    if cx.string_bytes(filename)?.contains(&0) {
        return Err(Fault::Wrong("filenamep", filename));
    }
    let path = super::fileio::expand_file_name(cx, filename)?;
    let multibyte = cx.string_view(filename)?.is_multibyte() || !path.is_ascii();
    let expanded = cx.make_string(path.into_bytes(), multibyte);
    symlink_expanded(cx, expanded, &[])
}}

defsubr! { name = "file-name-nondirectory", gnu = "fileio.c:534", effect = Global,
fn file_name_nondirectory(cx, filename: Value) -> Step {
    cx.string_bytes(filename)?;
    let operation = cx.intern("file-name-nondirectory");
    let handler = super::fileio::find_file_name_handler(cx, filename, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, filename], then: directory_file_returned, saved: vec![] });
    }
    let multibyte = cx.string_view(filename)?.is_multibyte();
    let bytes = cx.string_bytes(filename)?;
    let start = bytes.iter().rposition(|&b| b == b'/').map_or(0, |i| i + 1);
    let bytes = bytes[start..].to_vec();
    Ok(Step::Done(cx.make_string(bytes, multibyte)))
}}

defsubr! { name = "file-name-concat", gnu = "fileio.c:846", effect = Pure,
fn file_name_concat(cx, directory: Value; rest components) {
    // nil and "" are skipped; each part but the last gets a slash unless it
    // ends in one. Multibyte when any part is non-ASCII multibyte, a
    // unibyte non-ASCII part then converted as string-to-multibyte does.
    let mut parts = Vec::new();
    for &arg in std::iter::once(&directory).chain(components) {
        if arg == Value::NIL { continue; }
        let view = cx.string_view(arg).map_err(|_| Fault::Wrong("stringp", arg))?;
        if view.len() == 0 { continue; }
        parts.push((view.bytes().to_vec(), view.is_multibyte()));
    }
    let multibyte = parts.iter().any(|(bytes, multibyte)| *multibyte && !bytes.is_ascii());
    let mut out = Vec::new();
    let last = parts.len().saturating_sub(1);
    for (i, (bytes, part_multibyte)) in parts.into_iter().enumerate() {
        if multibyte && !part_multibyte {
            for b in bytes {
                let mut raw = [0; 5];
                let code = if b < 0x80 { u32::from(b) } else { crate::lstring::BYTE8_OFFSET + u32::from(b) };
                let n = crate::lstring::char_string(code, &mut raw);
                out.extend_from_slice(&raw[..n]);
            }
        } else {
            out.extend_from_slice(&bytes);
        }
        if i < last && out.last() != Some(&b'/') { out.push(b'/'); }
    }
    Ok(cx.make_string(out, multibyte))
}}

defsubr! { name = "file-name-as-directory", gnu = "fileio.c:634", effect = Global,
fn file_name_as_directory(cx, file: Value) -> Step {
    cx.string_bytes(file)?;
    let operation = cx.intern("file-name-as-directory");
    let handler = super::fileio::find_file_name_handler(cx, file, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, file], then: directory_file_returned, saved: vec![] });
    }
    let multibyte = cx.string_view(file)?.is_multibyte();
    let mut bytes = cx.string_bytes(file)?.to_vec();
    if bytes.is_empty() { bytes.extend_from_slice(b"./"); }
    else if bytes.last() != Some(&b'/') { bytes.push(b'/'); }
    Ok(Step::Done(cx.make_string(bytes, multibyte)))
}}

fn directory_file_returned(cx: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    if cx.string_bytes(value).is_ok() { Ok(Step::Done(value)) }
    else { Err(Fault::Error("Invalid handler in ‘file-name-handler-alist’".into())) }
}

defsubr! { name = "directory-file-name", gnu = "fileio.c:727", effect = Global,
fn directory_file_name(cx, directory: Value) -> Step {
    cx.string_bytes(directory)?;
    let operation = cx.intern("directory-file-name");
    let handler = super::fileio::find_file_name_handler(cx, directory, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, directory], then: directory_file_returned, saved: vec![] });
    }
    let multibyte = cx.string_view(directory)?.is_multibyte();
    let mut bytes = cx.string_bytes(directory)?.to_vec();
    if !(bytes.len() == 2 && bytes[0] == b'/') {
        while bytes.len() > 1 && bytes.last() == Some(&b'/') { bytes.pop(); }
    }
    Ok(Step::Done(cx.make_string(bytes, multibyte)))
}}

fn directory_returned(cx: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(if cx.string_bytes(value).is_ok() { value } else { Value::NIL }))
}

defsubr! { name = "file-name-directory", gnu = "fileio.c:509", effect = Global,
fn file_name_directory(cx, filename: Value) -> Step {
    cx.string_bytes(filename)?;
    let operation = cx.intern("file-name-directory");
    let handler = super::fileio::find_file_name_handler(cx, filename, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, filename], then: directory_returned, saved: vec![] });
    }
    let view = cx.string_view(filename)?;
    let multibyte = view.is_multibyte();
    let bytes = cx.string_bytes(filename)?;
    let Some(end) = bytes.iter().rposition(|&b| b == b'/') else {
        return Ok(Step::Done(Value::NIL));
    };
    let bytes = bytes[..=end].to_vec();
    Ok(Step::Done(cx.make_string(bytes, multibyte)))
}}

fn returned(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

/// A unibyte non-ASCII NAME is an encoded file name (ENCODE_FILE's output,
/// which `set-auto-coding-function' receives). VM2 file names are UTF-8, so
/// its bytes name the same file as the multibyte string they spell.
fn decoded_name(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<Value, Fault> {
    let view = cx.string_view(name)?;
    if view.is_multibyte() || view.bytes().is_ascii() { return Ok(name); }
    let bytes = view.bytes().to_vec();
    super::filename::utf8_file_names(cx, "file-name-case-insensitive-p")?;
    if std::str::from_utf8(&bytes).is_err() {
        return Err(Fault::Error("VM2 file-name-case-insensitive-p of a non-UTF-8 unibyte file name is unsupported".into()));
    }
    Ok(cx.make_string(bytes, true))
}

defsubr! { name = "file-name-case-insensitive-p", gnu = "fileio.c:2688", effect = Global,
fn file_name_case_insensitive_p(cx, filename: Value) -> Step {
    cx.string_bytes(filename)?;
    let filename = decoded_name(cx, filename)?;
    let expansion = cx.intern("expand-file-name");
    if super::fileio::find_file_name_handler(cx, filename, expansion)? != Value::NIL {
        return Err(Fault::Error("VM2 file-name-case-insensitive-p expand-file-name handler is unsupported".into()));
    }
    let path = super::fileio::expand_file_name(cx, filename)?;
    let multibyte = cx.string_view(filename)?.is_multibyte() || !path.is_ascii();
    let expanded = if cx.string_text(filename)? == path { filename }
        else { cx.make_string(path.into_bytes(), multibyte) };
    let operation = cx.intern("file-name-case-insensitive-p");
    let handler = super::fileio::find_file_name_handler(cx, expanded, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, expanded], then: returned, saved: vec![] });
    }
    let path = cx.string_text(expanded)?.to_owned();
    Ok(Step::Done(if folds_case(std::path::Path::new(&path)) { Value::TRUE } else { Value::NIL }))
}}

// Deliberately differs from pinned GNU, whose GNU/Linux answer is nil by
// platform assumption (fileio.c `file_name_case_insensitive_err'): the
// filesystem is measured, by its type and its directory's casefold flag. No
// directory is listed, so its size cannot matter, and every failure answers
// GNU's nil.
#[cfg(target_os = "linux")]
fn folds_case(path: &std::path::Path) -> bool {
    let Some(directory) = containing_directory(path) else { return false };
    classify_case(crate::host::filesystem_type(&directory), || crate::host::directory_flags(&directory))
}

#[cfg(not(target_os = "linux"))]
fn folds_case(_: &std::path::Path) -> bool {
    cfg!(windows)
}

/// PATH when it is a directory, else the directory its name is in: the
/// nearest that exists, as GNU's walk up the tree finds on trouble.
#[cfg(target_os = "linux")]
fn containing_directory(mut path: &std::path::Path) -> Option<std::path::PathBuf> {
    loop {
        match crate::host::entry_type(path) {
            Ok(t) if std::fs::FileType::is_dir(&t) => return Some(path.to_owned()),
            Ok(_) => return path.parent().map(std::path::Path::to_owned),
            Err(_) => path = path.parent()?,
        }
    }
}

// Filesystem magic numbers (statfs(2); linux/magic.h, else the driver).
const MSDOS_SUPER_MAGIC: u32 = 0x4d44; // vfat and msdos
const EXFAT_SUPER_MAGIC: u32 = 0x2011_bab0;
const NTFS_SB_MAGIC: u32 = 0x5346_544e; // fs/ntfs, case-insensitive unless case_sensitive=yes
const HFS_SUPER_MAGIC: u32 = 0x4244;
const HFSPLUS_SUPER_MAGIC: u32 = 0x482b;
const EXT4_SUPER_MAGIC: u32 = 0xef53; // ext2, ext3 and ext4
const F2FS_SUPER_MAGIC: u32 = 0xf2f5_2010;
const TMPFS_MAGIC: u32 = 0x0102_1994; // casefold since Linux 6.13
/// linux/fs.h: "case-insensitive name lookup" on this directory.
const FS_CASEFOLD_FL: u32 = 0x4000_0000;

/// Whether a filesystem of type FS_TYPE folds case: by its type, or, where
/// folding is a per-directory attribute, by the directory's FLAGS, asked
/// only there. Any error answers nil, as GNU does when it cannot tell.
fn classify_case(fs_type: std::io::Result<u32>, flags: impl FnOnce() -> std::io::Result<u32>) -> bool {
    match fs_type {
        Ok(MSDOS_SUPER_MAGIC | EXFAT_SUPER_MAGIC | NTFS_SB_MAGIC | HFS_SUPER_MAGIC | HFSPLUS_SUPER_MAGIC) => true,
        Ok(EXT4_SUPER_MAGIC | F2FS_SUPER_MAGIC | TMPFS_MAGIC) => flags().is_ok_and(|f| f & FS_CASEFOLD_FL != 0),
        _ => false,
    }
}

#[cfg(test)]
mod case_tests {
    use super::*;
    fn error(errno: i32) -> std::io::Result<u32> { Err(std::io::Error::from_raw_os_error(errno)) }
    fn unasked() -> std::io::Result<u32> { panic!("flags asked of a filesystem without casefold") }

    #[test]
    fn a_folding_filesystem_type_answers_t_without_flags() {
        for magic in [MSDOS_SUPER_MAGIC, EXFAT_SUPER_MAGIC, NTFS_SB_MAGIC, HFS_SUPER_MAGIC, HFSPLUS_SUPER_MAGIC] {
            assert!(classify_case(Ok(magic), unasked), "{magic:#x}");
        }
    }

    #[test]
    fn other_filesystem_types_answer_nil_without_flags() {
        // btrfs, xfs, ntfs3 (case-sensitive unless mounted nocase), CIFS,
        // SMB2 (the server and mount options decide), FUSE.
        for magic in [0x9123_683e, 0x5846_5342, 0x7366_746e, 0xff53_4d42, 0xfe53_4d42, 0x6573_5546] {
            assert!(!classify_case(Ok(magic), unasked), "{magic:#x}");
        }
    }

    #[test]
    fn a_casefold_capable_filesystem_answers_from_the_directory_flag() {
        for magic in [EXT4_SUPER_MAGIC, F2FS_SUPER_MAGIC, TMPFS_MAGIC] {
            assert!(classify_case(Ok(magic), || Ok(FS_CASEFOLD_FL | 0x80000)), "{magic:#x}");
            assert!(!classify_case(Ok(magic), || Ok(0x80000)), "{magic:#x}");
            for errno in [libc::ENOTTY, libc::EPERM, libc::EACCES, libc::ELOOP] {
                assert!(!classify_case(Ok(magic), || error(errno)), "{magic:#x} {errno}");
            }
        }
    }

    #[test]
    fn a_statfs_error_answers_nil() {
        for errno in [libc::EACCES, libc::ENOENT, libc::ENOSYS] {
            assert!(!classify_case(error(errno), unasked), "{errno}");
        }
    }
}
