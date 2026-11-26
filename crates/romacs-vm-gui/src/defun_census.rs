//! The GUI's loaded world supplies the live inventory of missing DEFUNs
//! and DEFVARs from the pinned GNU fixtures. Each gap needs a reason in
//! its own file. When the world gains a binding, its reason leaves with it.

#[cfg(test)]
mod tests {
    use crate::fonts::Display;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// The lisp/vm Lisp halves of prims/{keyboard,insdel,message,window_scroll,treesit,autosave,filename}.rs.
    const LAZY_HALVES: [&str; 8] = ["keyboard.el", "minibuf.el", "insdel.el", "message.el", "window-scroll.el", "treesit.el", "auto-save.el", "dired.el"];

    fn root() -> PathBuf { PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")) }

    /// Every name in fixtures/FIXTURE's KEY object: the DEFUNs or DEFVARs at the pin.
    fn gnu_names(fixture: &str, key: &str) -> Vec<String> {
        let text = std::fs::read_to_string(root().join("crates/romacs-vm/tests/fixtures").join(fixture)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json[key].as_object().unwrap().keys().cloned().collect()
    }

    fn fragments(path: &Path) -> BTreeMap<String, String> {
        fs::read_dir(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                assert!(entry.file_type().unwrap().is_file(), "{}: expected a fragment file", path.display());
                let name = entry.file_name().into_string()
                    .unwrap_or_else(|_| panic!("{}: expected a UTF-8 symbol name", path.display()));
                let text = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                (name, text)
            })
            .collect()
    }

    /// NAMES the GUI's loadup world leaves undefined: those PREDICATE
    /// (fboundp, boundp) refuses.
    fn undefined(names: Vec<String>, predicate: &'static str) -> Vec<String> {
        std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
            // The chain from a world, then the display, as a restored
            // launch attaches it (main.rs), and the session.
            let boot = crate::test_world::Boot { frame: (1200, 800, 24, 10), ..crate::test_world::Boot::LOADUP };
            let (mut vm, loadup) = crate::test_world::chain(boot);
            vm.set_display_host(Some(Box::new(Display::new().unwrap())));
            let home = crate::test_home::private_home(&mut vm, &format!("census-{predicate}"));
            let started = crate::startup::resume_loadup(&mut vm, crate::startup::Launch::Loadup, loadup).unwrap();
            crate::startup::finish_loadup(&mut vm, &started).unwrap();
            // The Lisp halves primitives install on first use (the command
            // loop, the minibuffer, insdel, messages, scrolling, treesit),
            // as a session's first commands install them.
            for half in LAZY_HALVES {
                vm.eval_source(&std::fs::read_to_string(root().join("lisp/vm").join(half)).unwrap()).unwrap();
            }
            let quoted: Vec<String> = names.iter().map(|n| serde_json::to_string(n).unwrap()).collect();
            // Written one name a line to a file, read back as UTF-8.
            let out = home.join("undefined.txt");
            vm.eval_source(&format!(
                "(with-temp-file {} (dolist (n (list {})) (unless ({predicate} (intern n)) (insert n \"\\n\"))))",
                serde_json::to_string(&out.display().to_string()).unwrap(), quoted.join(" "))).unwrap();
            let missing = std::fs::read_to_string(&out).unwrap();
            let _ = std::fs::remove_dir_all(&home);
            missing.lines().map(str::to_owned).collect()
        }).unwrap().join().unwrap()
    }

    #[test]
    fn every_gnu_defun_is_defined_or_in_the_census() {
        check_gaps("ledger/defun-gaps", undefined(gnu_names("gnu_defun_arity.json", "arity"), "fboundp"));
    }

    #[test]
    fn every_gnu_defvar_is_bound_or_in_the_census() {
        check_gaps("ledger/defvar-gaps", undefined(gnu_names("gnu_defvars.json", "defvars"), "boundp"));
    }

    fn check_gaps(dir: &str, missing: Vec<String>) {
        let problems = gap_problems(&missing.into_iter().collect(), &fragments(&root().join(dir)));
        assert!(problems.is_empty(), "{dir}:\n{}", problems.join("\n"));
    }

    fn gap_problems(missing: &BTreeSet<String>, fragments: &BTreeMap<String, String>) -> Vec<String> {
        let mut problems = vec![];
        for name in missing {
            if !fragments.contains_key(name) {
                problems.push(format!("{name}: missing from the world without an explanation"));
            }
        }
        for (name, reason) in fragments {
            if !missing.contains(name) {
                problems.push(format!("{name}: no longer a gap; remove its fragment"));
            }
            if reason.trim().is_empty() {
                problems.push(format!("{name}: empty explanation"));
            }
        }
        problems
    }

    #[test]
    fn gaps_need_reasons_and_recovered_symbols_leave_no_fragment() {
        let missing = ["accepted".into(), "empty".into(), "new".into()].into();
        let fragments = [
            ("accepted".into(), "Platform-specific primitive".into()),
            ("empty".into(), " \n".into()),
            ("recovered".into(), "Former gap".into()),
        ].into();
        assert_eq!(gap_problems(&missing, &fragments), [
            "new: missing from the world without an explanation",
            "empty: empty explanation",
            "recovered: no longer a gap; remove its fragment",
        ]);
    }
}
