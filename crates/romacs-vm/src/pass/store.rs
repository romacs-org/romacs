//! The store's entry names: each `NAME.gpg` under its directory, as pass(1)
//! lists them.  Hidden files and directories (`.git`, `.gpg-id`,
//! `.extensions`) are not entries; symbolic links are followed, as
//! pass's `tree -l` follows them, except into a directory the walk is
//! already inside.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The names of the entries under ROOT, sorted, each relative to ROOT,
/// `/`-separated and without `.gpg`.  An unreadable subdirectory has no
/// entries; ROOT itself must be readable.
pub(crate) fn entries(root: &Path) -> std::io::Result<Vec<String>> {
    let mut found = Vec::new();
    walk(root, "", &mut HashSet::new(), &mut found, true)?;
    found.sort_unstable();
    Ok(found)
}

/// DIR's entries under PREFIX into FOUND.  ANCESTORS are the directories
/// the walk is inside, by device and inode, so a link back to one of them
/// is not followed.
fn walk(dir: &Path, prefix: &str, ancestors: &mut HashSet<(u64, u64)>, found: &mut Vec<String>, top: bool) -> std::io::Result<()> {
    let identity = match crate::host::metadata(dir) {
        Ok(meta) => (meta.dev(), meta.ino()),
        Err(e) if top => return Err(e),
        Err(_) => return Ok(()),
    };
    if !ancestors.insert(identity) {
        return Ok(());
    }
    let walked = entries_of(dir, prefix, ancestors, found, top);
    ancestors.remove(&identity);
    walked
}

fn entries_of(dir: &Path, prefix: &str, ancestors: &mut HashSet<(u64, u64)>, found: &mut Vec<String>, top: bool) -> std::io::Result<()> {
    let listing = match crate::host::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if top => return Err(e),
        Err(_) => return Ok(()),
    };
    for item in listing.flatten() {
        let name = item.file_name();
        let Some(name) = name.to_str().filter(|name| !name.starts_with('.')) else { continue };
        let path = item.path();
        // Through a link to what it names, as `tree -l'.
        let Ok(meta) = crate::host::metadata(&path) else { continue };
        if meta.is_dir() {
            walk(&path, &format!("{prefix}{name}/"), ancestors, found, false)?;
        } else if let Some(stem) = name.strip_suffix(".gpg").filter(|stem| !stem.is_empty()) {
            found.push(format!("{prefix}{stem}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_the_gpg_files_by_relative_name() {
        let root = std::env::temp_dir().join(format!("romacs-pass-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["email", "xmpp/chat.example.org", ".git/objects", "empty"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["email/a.gpg", "xmpp/chat.example.org/me.gpg", "top.gpg", ".gpg-id",
                     "notes.txt", ".git/objects/x.gpg", ".hidden.gpg", ".gpg"] {
            std::fs::write(root.join(file), b"").unwrap();
        }
        std::os::unix::fs::symlink(root.join("email"), root.join("mail-link")).unwrap();
        // A link back into the walk is not followed.
        std::os::unix::fs::symlink(&root, root.join("email/loop")).unwrap();
        let names = entries(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(names, ["email/a", "mail-link/a", "top", "xmpp/chat.example.org/me"]);
        assert!(entries(Path::new("/nonexistent-romacs-pass-store")).is_err());
    }
}
