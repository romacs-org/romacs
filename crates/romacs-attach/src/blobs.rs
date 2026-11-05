//! `blobs`: a harness transcript's base64 images moved into a session's
//! store (attachments.md §5.3, the supervisor's extraction as a filter).
//! Every string `data` whose sibling `media_type` or `mimeType` names an
//! image or a PDF becomes `{"romacs-file": REL, "bytes": N}`, REL naming
//! the stored file from the store's parent (`attachments/<sha256>.<ext>`).
use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;

/// Whether LINE may hold a block to extract: key names appear verbatim
/// whatever the JSON's whitespace.
#[must_use]
pub fn may_hold(line: &str) -> bool {
    line.contains("\"media_type\"") || line.contains("\"mimeType\"")
}

fn extension(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        m if m.starts_with("image/") => "img",
        _ => return None,
    })
}

/// VALUE with its extractable blocks stored in STORE.
fn reduce(value: &mut Value, store: &Path) -> std::io::Result<()> {
    match value {
        Value::Object(object) => {
            let mime = object.get("media_type").or_else(|| object.get("mimeType")).and_then(Value::as_str);
            if let (Some(ext), Some(Value::String(data))) = (mime.and_then(extension), object.get("data"))
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
            {
                let stored = crate::store::store(store, &bytes, ext)?;
                let name = store.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let file = stored.path.file_name().expect("stored files have names").to_string_lossy().into_owned();
                object.insert("data".into(), json!({"romacs-file": format!("{name}/{file}"), "bytes": stored.bytes}));
            }
            for child in object.values_mut() {
                reduce(child, store)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                reduce(item, store)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// LINE with its extractable blocks stored in STORE and replaced by
/// references.  A line that cannot hold one, or is not JSON, comes back as
/// it is.
pub fn reduce_line(line: &str, store: &Path) -> std::io::Result<String> {
    if !may_hold(line) {
        return Ok(line.to_owned());
    }
    let Ok(mut value) = serde_json::from_str::<Value>(line) else { return Ok(line.to_owned()) };
    reduce(&mut value, store)?;
    Ok(value.to_string())
}
