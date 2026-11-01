//! The packaged desktop entry may only claim what the binary does.
//!
//! A file manager runs `Exec=` with `%f`/`%F`/`%u`/`%U` replaced by the
//! chosen files. If the binary rejects those arguments, "Open With Romacs"
//! fails silently from the user's side, so the claim is checked here against
//! the real binary, not against the text of the entry alone.

const ENTRY: &str = include_str!("../../romacs-display-runtime/assets/romacs.desktop");
const FILE_CODES: [&str; 4] = ["%f", "%F", "%u", "%U"];

fn exec_args() -> Vec<String> {
    let exec: Vec<&str> = ENTRY.lines().filter_map(|l| l.strip_prefix("Exec=")).collect();
    assert_eq!(exec.len(), 1, "desktop entry must have exactly one Exec= line");
    // Quoting and embedded field codes would need the spec's full grammar;
    // refuse them rather than model them wrongly.
    assert!(!exec[0].contains(['"', '\'', '\\']), "Exec= quoting is not modelled: {}", exec[0]);
    let mut words = exec[0].split_whitespace().map(str::to_owned);
    assert_eq!(words.next().as_deref(), Some("romacs"), "Exec must launch the packaged `romacs`");
    words.collect()
}

/// Expand field codes as a launcher would (Desktop Entry Spec, "The Exec key").
fn expand(args: &[String], file: &std::path::Path) -> Vec<String> {
    let mut out = vec![];
    for a in args {
        match a.as_str() {
            "%f" | "%F" | "%k" => out.push(file.to_string_lossy().into_owned()),
            "%u" | "%U" => out.push(format!("file://{}", file.display())),
            "%i" => out.extend(["--icon".to_owned(), "romacs".to_owned()]),
            "%c" => out.push("Romacs".to_owned()),
            code if code.contains('%') && code != "%%" => panic!("field code {code} not modelled"),
            word => out.push(word.replace("%%", "%")),
        }
    }
    out
}

/// Run the GUI binary with `args`, bounded. The Wayland socket does not
/// exist, so a command line the binary ACCEPTS ends its session at the display
/// connect with exactly that diagnostic; anything else means it was rejected.
fn accepted(args: &[String]) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("romacs-desktop-entry-{}-{}", std::process::id(), args.len()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("no-such-wayland-socket");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(args)
        .env("WAYLAND_DISPLAY", &socket)
        .env_remove("WAYLAND_SOCKET")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("{args:?}: still running after 30s"));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    // The session's one line saying why it ended (src/exit.rs).
    let expected = format!("Cannot connect to Wayland socket {}: ", socket.display());
    let end: Option<serde_json::Value> = stderr.strip_prefix("romacs-exit: ").and_then(|line| serde_json::from_str(line).ok());
    let said = end.as_ref().is_some_and(|end| end["reason"] == "startup-error" && end["status"] == 1
        && end["detail"].as_str().is_some_and(|detail| detail.starts_with(&expected)));
    if status.code() == Some(1) && said && stderr.lines().count() == 1 {
        Ok(())
    } else {
        Err(format!("{args:?}: exit {:?}, stderr {stderr:?}", status.code()))
    }
}

#[test]
fn desktop_entry_exec_arguments_are_accepted_by_the_binary() {
    let dir = std::env::temp_dir().join(format!("romacs-desktop-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("opened-from-file-manager.txt");
    std::fs::write(&file, "x\n").unwrap();
    // The predicate must tell acceptance from rejection, including a known
    // option with a missing or malformed value.
    for bad in [&["--no-such-romacs-option"][..], &["--geometry"], &["--geometry", "x"]] {
        let bad: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
        assert!(accepted(&bad).is_err(), "probe cannot observe rejection of {bad:?}");
    }
    // Exec lines the binary must refuse even though they pass a file: an
    // option it does not have, and the icon field code (`--icon romacs`).
    for bad in [&["--open", "%F"][..], &["%i", "%F"]] {
        let bad: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
        let args = expand(&bad, &file);
        assert!(accepted(&args).is_err(), "Exec={} was accepted: {args:?}", bad.join(" "));
    }
    let args = expand(&exec_args(), &file);
    let verdict = accepted(&args);
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(why) = verdict {
        panic!("Exec= is rejected by the binary: {why}");
    }
}

#[test]
fn desktop_entry_claims_mime_types_only_with_a_file_argument() {
    let takes_files = exec_args().iter().any(|a| FILE_CODES.contains(&a.as_str()));
    let mime = ENTRY.lines().any(|l| l.starts_with("MimeType="));
    assert!(!mime || takes_files, "MimeType= without a file field code in Exec=");
}

/// The binary visits a positional FILE (startup.rs `visit_at_startup`), so
/// "Open With Romacs" must be offered: Exec= takes the files a file manager
/// chose, and the entry names the types it opens.
#[test]
fn desktop_entry_offers_to_open_files() {
    assert!(exec_args().iter().any(|a| a == "%F"), "Exec= must pass the chosen files (%F)");
    let mime: Vec<&str> = ENTRY.lines().filter_map(|l| l.strip_prefix("MimeType=")).collect();
    assert_eq!(mime, ["text/plain;text/org;"], "MimeType= must name the text types romacs opens");
}
