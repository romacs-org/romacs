//! GNU get-file-buffer is expanded-name equality, NOT inode equality.
//! The GUI's stronger visit lookup is deliberately separately named.
use super::*;
use super::fileio::{expanded_path, file_name_value, refuse_handler};
use std::os::unix::ffi::OsStrExt;

defsubr! { name = "get-file-buffer", gnu = "buffer.c:493", effect = Reads,
fn get_file_buffer(cx, filename: Value) {
    let (path, expanded) = expanded_path(cx, filename)?;
    refuse_handler(cx, expanded, "get-file-buffer")?;
    let bytes = path.as_os_str().as_bytes();
    Ok(cx.file_visits().into_iter().find(|(_, name, _, _)| name == bytes)
        .map_or(Value::NIL, |(buffer, _, _, _)| buffer))
}}

defsubr! { name = "romacs-find-buffer-visiting", gnu = "files.el:2354", effect = Reads,
fn find_buffer_visiting(cx, filename: Value) {
    let (path, expanded) = expanded_path(cx, filename)?;
    refuse_handler(cx, expanded, "get-file-buffer")?;
    let visits = cx.file_visits();
    let bytes = path.as_os_str().as_bytes();
    if let Some((buffer, _, _, _)) = visits.iter().find(|(_, name, _, _)| name == bytes) {
        return Ok(*buffer);
    }
    refuse_handler(cx, expanded, "file-truename")?;
    // GNU files.el checks the saved truename before the inode fallback.
    let truename = crate::host::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let true_value = file_name_value(cx, truename.as_os_str().as_bytes());
    refuse_handler(cx, true_value, "file-truename")?;
    refuse_handler(cx, true_value, "abbreviate-file-name")?;
    let as_path = |bytes: &[u8]| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    if let Some((buffer, _, _, _)) = visits.iter().find(|(_, name, _, saved)| {
        saved.as_deref().map(as_path) == Some(truename.clone())
            && crate::host::canonicalize(as_path(name)).unwrap_or_else(|_| as_path(name)) == truename
    }) {
        return Ok(*buffer);
    }
    refuse_handler(cx, true_value, "file-attributes")?;
    let identity = crate::forwarded::file_identity(&path);
    for (buffer, name, visited, saved) in visits {
        if identity.is_some() && visited == identity {
            let candidate = file_name_value(cx, &name);
            refuse_handler(cx, candidate, "file-exists-p")?;
            let saved = file_name_value(cx, saved.as_deref().unwrap_or(&name));
            refuse_handler(cx, saved, "file-attributes")?;
            if crate::forwarded::file_identity(as_path(&name)) == identity { return Ok(buffer); }
        }
    }
    Ok(Value::NIL)
}}
