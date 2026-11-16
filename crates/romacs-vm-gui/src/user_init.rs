//! User configuration loaded after the editor's built-in policy.
use romacs_vm::Vm;
use std::path::{Path, PathBuf};

fn path_from(config: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    config.filter(|path| path.is_absolute())
        .or_else(|| home.map(|path| path.join(".config")))
        .map(|path| path.join("romacs/init.el"))
}

/// Resolve XDG configuration, falling back to ~/.config/romacs/init.el.
pub fn path() -> Option<PathBuf> {
    path_from(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
              std::env::var_os("HOME").map(PathBuf::from))
}

/// The session's init is PATH, as GNU's startup says whether or not it
/// exists: `user-init-file', and the directory beside it.
fn identify(vm: &mut Vm, path: &Path) -> Result<(), String> {
    let file = path.to_str().ok_or("init path is not UTF-8")?;
    let directory = path.parent().ok_or("init path has no parent")?;
    let directory = format!("{}/", directory.to_str().ok_or("init directory is not UTF-8")?);
    vm.eval_source(&format!("(setq user-init-file {} user-emacs-directory {})",
        serde_json::to_string(file).map_err(|error| error.to_string())?,
        serde_json::to_string(&directory).map_err(|error| error.to_string())?))?;
    Ok(())
}

/// A session with an init, even one not written yet, has somewhere to save
/// settings: name PATH, then load the saved settings, after Romacs's
/// defaults and before the init (lisp/session/romacs-saved-settings.el).
pub fn start_saved_settings(vm: &mut Vm, path: &Path) -> Result<(), String> {
    identify(vm, path)?;
    vm.eval_source("(when (fboundp 'romacs-saved-settings-start) (romacs-saved-settings-start))")?;
    Ok(())
}

/// Load PATH as Lisp source; an absent init is optional, other errors propagate.
pub fn load(vm: &mut Vm, path: &Path) -> Result<bool, String> {
    match path.metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("{}: {error}", path.display())),
        Ok(metadata) if !metadata.is_file() => return Err(format!("{} is not a file", path.display())),
        Ok(_) => {},
    }
    identify(vm, path)?;
    // Through Lisp where the session has it (lisp/session/romacs-init-load.el):
    // an error's backtrace is kept as it happens, for the warning that says
    // so.  Its answer is nil, or the error worded as the loader words one.
    let file = serde_json::to_string(path.to_str().ok_or("init path is not UTF-8")?).map_err(|error| error.to_string())?;
    let answer = vm.eval_source(&format!(
        "(if (fboundp 'romacs-init-load) (let ((said (romacs-init-load {file}))) (if (stringp said) (encode-coding-string said 'utf-8) said)) t)"))?;
    if answer.is_nil() { return Ok(true); }
    if !answer.is_t() { return Err(named(path, &String::from_utf8_lossy(answer.bytes()?))); }
    // The loader's error names the file already (`FILE: line N form M:
    // ...`); one that does not is named once here.
    vm.load_file(path).map_err(|error| named(path, &error.to_string()))?;
    Ok(true)
}

/// ERROR, an error loading PATH, naming PATH once.
fn named(path: &Path, error: &str) -> String {
    let file = path.display().to_string();
    if error.starts_with(&file) { error.to_owned() } else { format!("{file}: {error}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_init_error_names_its_file_once() {
        let path = Path::new("/home/user/.config/romacs/init.el");
        let loader = "/home/user/.config/romacs/init.el: line 3 form 2: (oops): (void-function oops)";
        assert_eq!(named(path, loader), loader);
        assert_eq!(named(path, "Permission denied"), "/home/user/.config/romacs/init.el: Permission denied");
    }

    #[test]
    fn xdg_paths_follow_the_standard_fallback() {
        assert_eq!(path_from(Some("/config".into()), Some("/home/user".into())), Some("/config/romacs/init.el".into()));
        assert_eq!(path_from(Some("relative".into()), Some("/home/user".into())), Some("/home/user/.config/romacs/init.el".into()));
        assert_eq!(path_from(None, None), None);
    }

    #[test]
    fn source_init_is_optional_and_overrides_existing_defaults() {
        let root = std::env::temp_dir().join(format!("romacs-user-init-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("init.el");
        let mut vm = Vm::new();
        assert!(!load(&mut vm, &root.join("absent.el")).unwrap());
        vm.eval_source("(defvar init-fixture-value 1)").unwrap();
        std::fs::write(&file, ";;; -*- lexical-binding: t -*-\n(setq init-fixture-value 42)\n").unwrap();
        assert!(load(&mut vm, &file).unwrap());
        assert_eq!(vm.eval_source("init-fixture-value").unwrap().display().unwrap(), "42");
        assert_eq!(vm.eval_source("(equal user-init-file load-file-name)").unwrap().display().unwrap(), "nil");
        std::fs::write(&file, "(error \"INIT_FIXTURE_FAILURE\")").unwrap();
        assert!(load(&mut vm, &file).unwrap_err().contains("INIT_FIXTURE_FAILURE"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
