//! The censuses of GNU's DEFUNs a Romacs session leaves undefined
//! (DEFUN-CENSUS.tsv at the repository root) and of its DEFVARs left
//! unbound (DEFVAR-CENSUS.tsv): every DEFUN in GNU's src/*.c at the Lisp
//! pin (crates/romacs-vm/tests/fixtures/gnu_defun_arity.json) is either
//! fboundp in the session's world, as the GUI's loadup makes it, or has
//! one line in its census, and every DEFVAR there (fixtures/
//! gnu_defvars.json) is boundp or has one line in its:
//!
//!   NAME<TAB>STATUS<TAB>NOTE
//!
//! STATUS `needed' (a command, or called by shipped Lisp: to implement;
//! NOTE says by whom), `elsewhere' (in progress in another lane; NOTE says
//! which) or `absent' (deliberately left out; NOTE is the reason, which it
//! must give). Lines are sorted by name and `#' starts a comment.
//!
//! Each census is a ratchet: a listed name the world defines fails until
//! its line is removed, and an undefined DEFUN with no line fails until it
//! is classified. ROMACS_CENSUS_BLESS=1 rewrites the file to the world's
//! undefined names, keeping each kept name's line and adding new ones as
//! `needed'.

#[cfg(test)]
mod tests {
    use crate::fonts::Display;
    use romacs_vm::Vm;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const STATUSES: [&str; 3] = ["needed", "elsewhere", "absent"];
    /// The rlisp/vm Lisp halves of prims/{keyboard,insdel,message,window_scroll,treesit,autosave,filename}.rs.
    const LAZY_HALVES: [&str; 9] = ["keyboard.el", "minibuf.el", "window-change.el", "insdel.el", "message.el", "window-scroll.el", "treesit.el", "auto-save.el", "dired.el"];

    fn root() -> PathBuf { PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")) }

    /// Every name in fixtures/FIXTURE's KEY object: the DEFUNs or DEFVARs at the pin.
    fn gnu_names(fixture: &str, key: &str) -> Vec<String> {
        let text = std::fs::read_to_string(root().join("crates/romacs-vm/tests/fixtures").join(fixture)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json[key].as_object().unwrap().keys().cloned().collect()
    }

    /// The census's lines, by name: (STATUS, NOTE).
    fn census(text: &str) -> BTreeMap<String, (String, String)> {
        text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields.next().unwrap().to_owned();
            (name, (fields.next().unwrap_or("").to_owned(), fields.next().unwrap_or("").to_owned()))
        }).collect()
    }

    /// NAMES the GUI's loadup world leaves undefined: those PREDICATE
    /// (fboundp, boundp) refuses.
    fn undefined(names: Vec<String>, predicate: &'static str) -> Vec<String> {
        std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 24, 10).unwrap();
            vm.set_display_host(Some(Box::new(Display::new().unwrap())));
            let home = std::env::temp_dir().join(format!("romacs-census-{predicate}-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            vm.eval_source(&format!("(setq process-environment (append (list \"HOME={0}\" \"XDG_CONFIG_HOME={0}/config\") process-environment))",
                                    home.display())).unwrap();
            let started = crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::Loadup, None).unwrap();
            crate::startup::finish_loadup(&mut vm, &started).unwrap();
            // The Lisp halves primitives install on first use (the command
            // loop, the minibuffer, insdel, messages, scrolling, treesit),
            // as a session's first commands install them.
            for half in LAZY_HALVES {
                vm.eval_source(&std::fs::read_to_string(root().join("rlisp/vm").join(half)).unwrap()).unwrap();
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
        check_census("DEFUN-CENSUS.tsv", undefined(gnu_names("gnu_defun_arity.json", "arity"), "fboundp"));
    }

    #[test]
    fn every_gnu_defvar_is_bound_or_in_the_census() {
        check_census("DEFVAR-CENSUS.tsv", undefined(gnu_names("gnu_defvars.json", "defvars"), "boundp"));
    }

    /// The census at FILE against the MISSING names: blessed, or checked.
    fn check_census(file: &str, missing: Vec<String>) {
        let path = root().join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let listed = census(&text);
        if std::env::var("ROMACS_CENSUS_BLESS").is_ok_and(|v| v == "1") {
            let header: String = text.lines().take_while(|l| l.starts_with('#')).map(|l| format!("{l}\n")).collect();
            let body: String = missing.iter().map(|name| match listed.get(name) {
                Some((status, note)) => format!("{name}\t{status}\t{note}\n"),
                None => format!("{name}\tneeded\t\n"),
            }).collect();
            std::fs::write(&path, header + &body).unwrap();
            return;
        }
        let missing_set: std::collections::BTreeSet<&String> = missing.iter().collect();
        let mut problems = vec![];
        for name in &missing {
            if !listed.contains_key(name) { problems.push(format!("{name}: undefined and not in the census; classify it")); }
        }
        for (name, (status, note)) in &listed {
            if !missing_set.contains(name) { problems.push(format!("{name}: defined now; remove its line")); }
            if !STATUSES.contains(&status.as_str()) { problems.push(format!("{name}: status {status:?} is none of {STATUSES:?}")); }
            if status == "absent" && note.trim().is_empty() { problems.push(format!("{name}: absent without a reason")); }
        }
        let names: Vec<&String> = text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.split('\t').next().unwrap()).map(|n| listed.get_key_value(n).unwrap().0).collect();
        if names.windows(2).any(|w| w[0] >= w[1]) { problems.push("lines are not sorted by name, once each".into()); }
        assert!(problems.is_empty(), "{file} (ROMACS_CENSUS_BLESS=1 rewrites it):\n{}", problems.join("\n"));
    }
}
