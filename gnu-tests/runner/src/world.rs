//! `--world-trace OUT.json` (DESIGN-M1 §5, §13 step 2): run the recipe, the
//! preloads, harness and adapter, as a world build would, and write its read
//! trace. The process re-executes itself with the pinned per-process
//! environment, confined, in build mode; the child lists every refusal
//! rather than stopping at the first, writes the trace to OUT and exits 1 if
//! there was any.
use romacs_vm::host::{self, Pinned};
use romacs_vm::{Vm, world::sandbox};
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The pinned answers that no environment entry carries, until restore's
/// recipe digest derives them (§5 step 1).
const PID: u32 = 4242;
const SEED: [u8; 4] = [0; 4];

/// The child's environment: INVOKING without its per-process names, which
/// the pinned table under DIR supplies (§5 step 1). Keyed names keep their
/// values.
pub fn pinned_env(invoking: &[(OsString, OsString)], dir: &Path) -> Vec<(OsString, OsString)> {
    let p = dir.join("p");
    let pinned = [
        ("PWD", p.join("cwd")), ("TMPDIR", p.join("tmp")), ("TMP", p.join("tmp")),
        ("TEMP", p.join("tmp")), ("PATH", p.join("bin")), ("TERM", PathBuf::from("dumb")),
    ];
    invoking.iter()
        .filter(|(name, _)| !host::P_ENV.iter().any(|p| name == p))
        .cloned()
        .chain(pinned.map(|(name, value)| (OsString::from(name), value.into_os_string())))
        .collect()
}

/// The directory of OUT's pinned table: `OUT.d/p/{cwd,tmp,bin}`, empty and
/// read-only.
fn pinned_dir(out: &Path) -> PathBuf {
    let mut dir = out.as_os_str().to_owned();
    dir.push(".d");
    PathBuf::from(dir)
}

/// The flags whose value names a file or directory.
const PATH_FLAGS: [&str; 7] = ["--load-path", "--preload", "--harness", "--plan", "--reporter", "--audit", "--world-trace"];

/// ARGS, flag and value pairs as `parse` reads them, with each relative
/// path value resolved against CWD: the child runs elsewhere.
fn absolute_args(args: &[OsString], cwd: &Path) -> Vec<OsString> {
    args.chunks(2).flat_map(|pair| match pair {
        [flag, value] if PATH_FLAGS.iter().any(|f| flag == f) => vec![flag.clone(), cwd.join(value).into_os_string()],
        other => other.to_vec(),
    }).collect()
}

/// The parent: make the pinned directories and OUT, run the child with the
/// pinned environment in `p/cwd`, and exit as it does. The child creates no
/// file: its effects refuse, and OUT exists.
pub fn trace(out: &Path) -> ! {
    let status = host::current_dir().and_then(|cwd| {
        let out = cwd.join(out);
        let dir = pinned_dir(&out);
        host::create_dir_all(dir.join("p"))?;
        for name in ["cwd", "tmp", "bin"] {
            let path = dir.join("p").join(name);
            if !host::probe(host::Probe::IsDir, &path) {
                host::create_dir(&path, 0o500)?;
            }
        }
        host::write(&out, b"")?;
        let mut args = absolute_args(&std::env::args_os().skip(1).collect::<Vec<_>>(), &cwd);
        args.extend([OsString::from("--world-trace-child"), dir.clone().into_os_string()]);
        host::command(host::current_exe()?)?
            .args(args).env_clear().envs(pinned_env(&host::env_snapshot(), &dir))
            .current_dir(dir.join("p/cwd")).status()
    });
    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(70)),
        Err(e) => {
            crate::emit(json!({"event": "setup", "ok": false, "phase": "world-trace", "error": e.to_string()}));
            std::process::exit(3)
        }
    }
}

/// The child, before the VM is made: confine to the RECIPE's roots, the
/// pinned table under DIR and the binary's own, writing only OUT; then
/// record.
pub fn begin(recipe: Vec<PathBuf>, out: &Path, dir: &Path) {
    if let Err(e) = sandbox::confine(&sandbox::build_roots(recipe, dir), &[out.to_owned()]) {
        crate::emit(json!({"event": "setup", "ok": false, "phase": "world-confine", "error": e.to_string()}));
        std::process::exit(3);
    }
    host::enter_build(Pinned { clock: host::clock(), pid: PID, seed: SEED });
}

/// The child, once the recipe ran: check the startup inputs, write the
/// trace to OUT and exit, 1 if anything was refused.
pub fn finish(vm: &mut Vm, out: &Path) -> ! {
    vm.world_check_inputs();
    let trace = host::leave_build().expect("a world trace is recording");
    let count = |list: &str| trace[list].as_array().map_or(0, Vec::len);
    let (records, refusals) = (count("records"), count("refusals"));
    let written = serde_json::to_vec_pretty(&trace).map_err(std::io::Error::other).and_then(|bytes| host::write(out, bytes));
    crate::emit(json!({"event": "world-trace", "out": out, "records": records, "refusals": refusals,
                       "error": written.as_ref().err().map(ToString::to_string)}));
    std::process::exit(if written.is_err() { 3 } else if refusals > 0 { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_env_replaces_per_process_names_and_keeps_keyed_ones() {
        let env = |pairs: &[(&str, &str)]| pairs.iter().map(|(n, v)| (OsString::from(n), OsString::from(v))).collect::<Vec<_>>();
        let invoking = env(&[("HOME", "/home/u"), ("PATH", "/usr/bin"), ("TMPDIR", "/run/t"), ("DISPLAY", ":0"),
                             ("LANG", "C.UTF-8"), ("OLDPWD", "/x"), ("TERM", "xterm"), ("XDG_CONFIG_HOME", "/home/u/.config")]);
        let pinned = pinned_env(&invoking, Path::new("/s/out.json.d"));
        assert_eq!(pinned, env(&[
            ("HOME", "/home/u"), ("LANG", "C.UTF-8"), ("XDG_CONFIG_HOME", "/home/u/.config"),
            ("PWD", "/s/out.json.d/p/cwd"), ("TMPDIR", "/s/out.json.d/p/tmp"), ("TMP", "/s/out.json.d/p/tmp"),
            ("TEMP", "/s/out.json.d/p/tmp"), ("PATH", "/s/out.json.d/p/bin"), ("TERM", "dumb"),
        ]));
        assert_eq!(pinned_dir(Path::new("/s/out.json")), Path::new("/s/out.json.d"));
    }

    #[test]
    fn relative_paths_resolve_against_the_invoking_directory() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        let invoked = args(&["--plan", "plan.json", "--selector", "--plan", "--preload", "../p.el", "--load-path", "/abs",
                             "--harness", "h.el", "--world-trace", "out.json", "--harness-kind", "real"]);
        assert_eq!(absolute_args(&invoked, Path::new("/w")), args(&[
            "--plan", "/w/plan.json", "--selector", "--plan", "--preload", "/w/../p.el", "--load-path", "/abs",
            "--harness", "/w/h.el", "--world-trace", "/w/out.json", "--harness-kind", "real"]));
    }
}
