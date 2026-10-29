//! GNU get-file-buffer is expanded-name equality, NOT inode equality.
//! The GUI's stronger visit lookup is deliberately separately named.
use super::*;
use super::fileio::{expand_file_name, refuse_handler};

defsubr! { name = "get-file-buffer", gnu = "buffer.c:493", effect = Reads,
fn get_file_buffer(cx, filename: Value) {
    let path = expand_file_name(cx, filename)?;
    let expanded = cx.string(&path);
    refuse_handler(cx, expanded, "get-file-buffer")?;
    Ok(cx.file_visits().into_iter().find(|(_, name, _, _)| *name == path)
        .map_or(Value::NIL, |(buffer, _, _, _)| buffer))
}}

defsubr! { name = "romacs-find-buffer-visiting", gnu = "files.el:2354", effect = Reads,
fn find_buffer_visiting(cx, filename: Value) {
    let path = expand_file_name(cx, filename)?;
    let expanded = cx.string(&path);
    refuse_handler(cx, expanded, "get-file-buffer")?;
    let visits = cx.file_visits();
    if let Some((buffer, _, _, _)) = visits.iter().find(|(_, name, _, _)| *name == path) {
        return Ok(*buffer);
    }
    refuse_handler(cx, expanded, "file-truename")?;
    // GNU files.el checks the saved truename before the inode fallback.
    let truename = crate::host::canonicalize(&path).ok()
        .and_then(|p| p.to_str().map(str::to_owned)).unwrap_or_else(|| path.clone());
    let true_value = cx.string(&truename);
    refuse_handler(cx, true_value, "file-truename")?;
    refuse_handler(cx, true_value, "abbreviate-file-name")?;
    if let Some((buffer, _, _, _)) = visits.iter().find(|(_, name, _, saved)| {
        saved.as_ref() == Some(&truename)
            && crate::host::canonicalize(name).ok().and_then(|p| p.to_str().map(str::to_owned))
                .unwrap_or_else(|| name.clone()) == truename
    }) {
        return Ok(*buffer);
    }
    refuse_handler(cx, true_value, "file-attributes")?;
    let identity = crate::forwarded::file_identity(&path);
    for (buffer, name, visited, saved) in visits {
        if identity.is_some() && visited == identity {
            let candidate = cx.string(&name);
            refuse_handler(cx, candidate, "file-exists-p")?;
            let saved = cx.string(saved.as_deref().unwrap_or(&name));
            refuse_handler(cx, saved, "file-attributes")?;
            if crate::forwarded::file_identity(&name) == identity { return Ok(buffer); }
        }
    }
    Ok(Value::NIL)
}}
