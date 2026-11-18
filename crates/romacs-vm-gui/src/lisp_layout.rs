//! The Lisp tree's layout, from `lisp/bundled.json` (`tools/lib/lisp_layout.py` reads the
//! same file for the Python tools). Pure: no VM and no other module of the crate, so a
//! file that includes `loadup.rs` by path can include this one beside it.

/// The Lisp tree's layout, declared once in `lisp/bundled.json` for the
/// load-path (`startup::runtime_load_path`), GNU's subdirs port (`loadup`),
/// the bundled autoloads (`lisp/session/romacs-bundled-autoloads.el`, which
/// reads the installed copy) and `tools/test-ert`: Romacs's subsystem
/// directories in load-path order, the packages' directory, and the
/// directories on no load path.
pub struct Layout {
    pub directories: Vec<String>,
    pub packages: String,
    pub off_path: Vec<String>,
}

/// `bundled.json` of the Lisp tree LISP (a runtime tree's `lisp/`) as JSON,
/// as the tree holds it now: never a copy compiled in.  Read through the
/// host, so a world built with it goes stale when it changes.
///
/// # Errors
/// When the file cannot be read or is not JSON.
pub fn bundled_layout(lisp: &std::path::Path) -> Result<serde_json::Value, String> {
    let file = lisp.join("bundled.json");
    let text = romacs_vm::host::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
}

/// The layout the Lisp tree LISP's `bundled.json` declares.
///
/// # Errors
/// When the file cannot be read, lacks a key or a key's value has the
/// wrong shape.
pub fn layout(lisp: &std::path::Path) -> Result<Layout, String> {
    let json = bundled_layout(lisp)?;
    let names = |key: &str| -> Result<Vec<String>, String> {
        json[key].as_array().ok_or(format!("lisp/bundled.json: no {key}"))?.iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or(format!("lisp/bundled.json: malformed {key}"))).collect()
    };
    Ok(Layout {
        directories: names("directories")?,
        packages: json["packages"].as_str().ok_or("lisp/bundled.json: no packages")?.to_owned(),
        off_path: names("off_path")?,
    })
}

/// The names under `lisp/` that are Romacs's, not GNU's directories: its
/// subsystems, the packages' directory and those on no load-path.
///
/// # Errors
/// As [`layout`].
pub fn own_directories(lisp: &std::path::Path) -> Result<Vec<String>, String> {
    let layout = layout(lisp)?;
    Ok(layout.directories.into_iter().chain(layout.off_path).chain([layout.packages]).collect())
}
