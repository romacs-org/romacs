//! Launch: VM2's own workspace by default; `--loadup` runs GNU's loadup.el
//! with romacs stand-ins only where GNU's own command did not load, and
//! `--gnu-lisp` runs loadup.el alone. All visible results come from this VM.
use romacs_vm::Vm;

/// How the GUI starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Launch {
    /// `--loadup`: GNU's `loadup.el` in its own order ([`crate::loadup`]), then
    /// a romacs stand-in for each journey key whose GNU command loadup did not
    /// define, then the Rum workspace. Not the default: see main.rs.
    Loadup,
    /// `--gnu-lisp`: GNU's `loadup.el` and nothing romacs adds: no stand-ins,
    /// no Rum faces, no workspace. Shows what GNU's Lisp alone gives today.
    GnuOnly,
    /// Default (also `--no-gnu-lisp`): VM2's own workspace with no GNU Lisp
    /// and every stand-in.
    Workspace,
}

/// Every romacs stand-in: (key, GNU's command for it in GNU's global map,
/// romacs's command). After loadup, a key gets GNU's command only if loadup
/// DEFINED it (it was unbound, or bound to something else, before loadup ran
/// GNU's files); otherwise the romacs command, which names itself
/// `romacs-...` in every command reply so it can never read as GNU's.
pub const STAND_INS: [(&str, &str, &str); 13] = [
    ("[14]", "next-line", "romacs-gui-next-line"),
    ("[down]", "next-line", "romacs-gui-next-line"),
    ("[16]", "previous-line", "romacs-gui-previous-line"),
    ("[up]", "previous-line", "romacs-gui-previous-line"),
    ("[24 50]", "split-window-below", "romacs-gui-split-below"),
    ("[24 51]", "split-window-right", "romacs-gui-split-right"),
    ("[24 48]", "delete-window", "romacs-gui-delete-window"),
    ("[24 49]", "delete-other-windows", "romacs-gui-delete-other-windows"),
    ("[24 111]", "other-window", "romacs-gui-other-window"),
    ("[24 3]", "save-buffers-kill-terminal", "romacs-gui-quit"),
    ("[24 5]", "eval-last-sexp", "romacs-gui-eval-last-sexp"),
    ("[21]", "universal-argument", "romacs-universal-argument"),
    // GNU binds C-x C-s to save-buffer in ctl-x-map (files.el:9323); GNU's
    // files.el does not load yet, so this is romacs-gui-save-buffer over the
    // C `write-region' (startup.el).
    ("[24 19]", "save-buffer", "romacs-gui-save-buffer"),
];

/// Files named on the command line (`romacs FILE...`, the desktop entry's
/// `%F`), visited in order once the workspace exists; the last one is shown.
static FILES: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Queue FILE for [`visit_queued`]. Relative names stay relative: the
/// evaluator expands them against the process directory (DIVERGENCES.md
/// VM2-PATH2), which is where `romacs FILE` was typed.
pub fn visit_at_startup(file: &str) {
    FILES.lock().unwrap().push(file.into());
}

/// Take every file [`visit_at_startup`] queued, leaving the queue empty.
fn take_queued() -> Vec<std::path::PathBuf> {
    std::mem::take(&mut *FILES.lock().unwrap())
}

/// Visit FILES in order into the selected window with
/// `romacs-gui-visit-file` (startup.el). A file the evaluator refuses
/// (VM2-FILEIO1: not UTF-8, CRLF, ...) is reported in `*romacs visit errors*`
/// and the session continues; a missing file becomes an empty buffer visiting
/// it, as GNU's find-file does.
pub fn visit_files(vm: &mut Vm, files: &[std::path::PathBuf]) -> Result<(), String> {
    let mut position = None;
    let mut literal = false;
    for file in files {
        if !literal {
            if file == std::path::Path::new("--") {
                literal = true;
                continue;
            }
            if let Some(value) = file.to_str().and_then(|s| s.strip_prefix('+')) {
                let parsed = value.split_once(':').map_or_else(
                    || parse_position_number(value).map(|line| (line, 0)),
                    |(line, column)| parse_position_number(line).zip(parse_position_number(column)),
                );
                if parsed.is_some() {
                    position = parsed;
                    continue;
                }
            }
        }
        let normalized = file.canonicalize().or_else(|_| {
            let absolute = if file.is_absolute() { file.clone() } else { std::env::current_dir()?.join(file) };
            Ok::<_, std::io::Error>(absolute.parent().and_then(|p| p.canonicalize().ok())
                .zip(absolute.file_name()).map(|(p, n)| p.join(n)).unwrap_or(absolute))
        }).map_err(|e| e.to_string())?;
        let file = &normalized;
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty()).unwrap_or_else(|| file.to_string_lossy().into_owned());
        let path = file.to_str().ok_or_else(|| format!("{}: file name is not UTF-8", file.display()))?;
        vm.eval_source(&format!("(romacs-gui-visit-file {} {})", lisp_string(path), lisp_string(&name)))?;
        if let Some((line, column)) = position.take() {
            if line > 0 {
                vm.eval_source(&format!("(goto-char (point-min)) (forward-line {})", line - 1))?;
            }
            if column > 0 { visit_column(vm, column - 1)?; }
            vm.eval_source("(set-window-point (selected-window) (point))")?;
        }
    }
    Ok(())
}

fn parse_position_number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
    value.parse().ok()
}

fn visit_column(vm: &mut Vm, target: u64) -> Result<(), String> {
    use romacs_vm::coordinates::ByteOffset;
    vm.eval_source("(beginning-of-line)")?;
    let value = vm.eval_source("(let ((start (point))) (end-of-line) (prog1 (buffer-substring start (point)) (goto-char start)))")?.typed_value()?;
    let line = value["value"].as_str().ok_or("startup column requires Unicode text")?;
    let tab = vm.eval_source("(if (boundp 'tab-width) tab-width 8)")?.as_integer()?.max(1) as u64;
    let mut column = 0;
    let mut characters = 0;
    for character in line.chars() {
        if column >= target { break; }
        let width = if character == '\t' { tab - column % tab }
        else if character.is_ascii_control() { 2 }
        else {
            let mut encoded = [0; 4];
            let scalar = character.encode_utf8(&mut encoded);
            let text = romacs_vm::text::Text::new(scalar.as_bytes())?;
            text.display_width(ByteOffset(0)..ByteOffset(scalar.len()))?.0 as u64
        };
        column += width;
        characters += 1;
    }
    vm.eval_source(&format!("(forward-char {characters})"))?;
    Ok(())
}

/// Which command each stand-in key ended up with.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub key: &'static str,
    pub gnu: &'static str,
    /// True: GNU's command, defined by GNU's Lisp during loadup.
    pub gnu_defined: bool,
}

/// What the launch did, for the status line and the log.
#[derive(Debug)]
pub struct Started {
    pub launch: Launch,
    pub loadup: Option<crate::loadup::Loadup>,
    pub bindings: Vec<Binding>,
    /// True when `mode-line-format` has a default binding after loadup,
    /// including nil or an unchanged pre-existing value, which is left alone;
    /// false when each workspace buffer shows the labelled
    /// [`STAND_IN_MODE_LINE`] (or, under --gnu-lisp, no mode line at all).
    pub gnu_mode_line: bool,
}

impl Started {
    pub fn status(&self) -> String {
        let mut text = match &self.loadup {
            Some(l) => l.status(),
            None => "GNU Lisp not loaded (the default; --loadup runs loadup.el); every journey key is a romacs stand-in".into(),
        };
        if let Some(stop) = self.loadup.as_ref().and_then(|l| l.gnu_would_stop()) {
            text.push_str(&format!("; GNU's own dump would have stopped at {stop}"));
        }
        if self.launch != Launch::GnuOnly {
            let gnu = self.bindings.iter().filter(|b| b.gnu_defined).count();
            text.push_str(&format!("; journey keys: {gnu} GNU, {} romacs stand-in", self.bindings.len() - gnu));
        }
        text
    }
}

/// How many leaf windows, from the selected one on, in the GUI's own window
/// order: the host's paint reads it to find the selected window's snapshot.
pub const SELECTED_TAIL: &str = "(length (memq (selected-window) (romacs-gui-leaves (romacs-gui-root))))";

/// GNU's `lisp/` for this process; see [`lisp_dir_from`].
pub fn lisp_dir() -> std::path::PathBuf {
    lisp_dir_from(std::env::var_os("ROMACS_LISP_DIR"), std::env::var_os("ROMACS_RUNTIME_ROOT"))
}

/// `ROMACS_LISP_DIR`, else `ROMACS_RUNTIME_ROOT/gnu-lisp` (an installed
/// `share/romacs`, which the Nix wrapper sets), else the checkout this binary
/// was built from. The last is a compile-time path: in a Nix build it names
/// the builder's `/build/source`, which no installed system has.
fn lisp_dir_from(lisp_dir: Option<std::ffi::OsString>, runtime_root: Option<std::ffi::OsString>) -> std::path::PathBuf {
    lisp_dir.map(Into::into)
        .or_else(|| runtime_root.map(|root| std::path::PathBuf::from(root).join("gnu-lisp")))
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gnu-lisp"))
}

/// GNU libraries precede Romacs libraries and their temporary dependencies.
/// VM helper sources are compile-time inputs and stay off the public path.
pub fn runtime_load_path(dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    fn root_and_children(root: &std::path::Path, path: &mut Vec<std::path::PathBuf>, excluded: &[&str]) -> Result<(), String> {
        path.push(root.to_path_buf());
        let mut children = Vec::new();
        for entry in std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))? {
            let entry = entry.map_err(|e| e.to_string())?.path();
            if entry.is_dir() && !excluded.iter().any(|name| entry.file_name() == Some(std::ffi::OsStr::new(name))) {
                children.push(entry);
            }
        }
        children.sort();
        path.extend(children);
        Ok(())
    }
    let mut path = Vec::new();
    root_and_children(dir, &mut path, &[])?;
    if let Some(owned) = dir.parent().map(|p| p.join("rlisp")).filter(|p| p.is_dir()) {
        root_and_children(&owned, &mut path, &["vm", "deps"])?;
        let deps = owned.join("deps");
        if deps.is_dir() {
            root_and_children(&deps, &mut path, &[])?;
        }
    }
    Ok(path)
}

const GUI_SUPPORT: &str = "(setq romacs-gui-quit-requested nil)
(fset 'romacs-gui--window-child (symbol-function 'window-child))
(defun romacs-gui-root ()
  (let ((w (selected-window))) (while (window-parent w) (setq w (window-parent w))) w))
(defun romacs-gui-leaves (w)
  (if (window-live-p w) (list w)
    (let ((child (romacs-gui--window-child w)) (leaves nil))
      (while child
        (setq leaves (append leaves (romacs-gui-leaves child)))
        (setq child (window-next-sibling child)))
      leaves)))";

/// Bind a key in the same global map used by command dispatch.
fn define_global(key: &str, command: &str) -> String {
    format!("(define-key (current-global-map) {key} '{command})")
}

/// Snapshot, before GNU's files load, what each GNU command name meant, so
/// "loadup defined it" is decided by identity, not by `fboundp` (VM2 defines
/// some GNU names natively, and those are not GNU's Lisp).
fn snapshot_gnu_commands(vm: &mut Vm) -> Result<(), String> {
    // No mapcar: VM2's Lisp prelude is not installed yet in this order.
    let entries: Vec<String> = STAND_INS.iter()
        .map(|s| format!("(cons '{0} (and (fboundp '{0}) (symbol-function '{0})))", s.1)).collect();
    vm.eval_source(&format!("(setq romacs-gui--before-loadup (list {}))", entries.join(" ")))?;
    Ok(())
}

fn gnu_defined(vm: &mut Vm, command: &str) -> Result<bool, String> {
    let v = vm.eval_source(&format!(
        "(and (fboundp '{command}) (not (eq (symbol-function '{command}) (cdr (assq '{command} romacs-gui--before-loadup)))))"
    ))?;
    Ok(v != romacs_vm::Fixed::NIL)
}

/// Bind every stand-in key: GNU's command where loadup defined it, else romacs's.
fn bind_journey_keys(vm: &mut Vm, after_loadup: bool) -> Result<Vec<Binding>, String> {
    // Install VM2's keymaps through a keymap primitive BEFORE any form names
    // `global-map`: VM2's lazy keymap setup treats an interned `global-map`
    // symbol as "already installed", and `global-set-key` then panics on the
    // missing internal map (crates/romacs-vm/src/keymap.rs ensure_keymaps).
    vm.eval_source("(keymapp nil)")?;
    let mut bindings = vec![];
    for (key, gnu, romacs) in STAND_INS {
        let gnu_defined = after_loadup && gnu_defined(vm, gnu)?;
        vm.eval_source(&define_global(key, if gnu_defined { gnu } else { romacs }))?;
        bindings.push(Binding { key, gnu, gnu_defined });
    }
    vm.eval_source(&define_global("[24 107]", "romacs-gui-kill-buffer"))?;
    vm.eval_source(&define_global("[24 6]", "find-file"))?;
    vm.eval_source(&define_global("[24 98]", "switch-to-buffer"))?;
    vm.eval_source(&define_global("[24 100]", "dired"))?;
    vm.eval_source(&define_global("[27 120]", "execute-extended-command"))?;
    vm.eval_source(&define_global("[134217848]", "execute-extended-command"))?;
    Ok(bindings)
}

/// Install Rum faces, editing commands, and two real buffers in an initial
/// split, on VM2's own runtime with NO GNU Lisp (`--no-gnu-lisp`). The host
/// must set its frame geometry first. After command dispatch it should
/// inspect `romacs-gui-quit-requested`: non-nil requests host shutdown.
pub fn initialize(vm: &mut Vm) -> Result<Started, String> {
    initialize_files(vm, &take_queued())
}

/// [`initialize`] visiting FILES (`romacs FILE...`) instead of the queue.
pub fn initialize_files(vm: &mut Vm, files: &[std::path::PathBuf]) -> Result<Started, String> {
    vm.eval_source(include_str!("../../../rlisp/romacs-startup.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-find-file.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-buffer-switch.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-extended-command.el"))?;
    initialize_editing(vm)?;
    vm.eval_source(include_str!("../../../rlisp/romacs-dired.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-term.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-install-command.el"))?;
    crate::eval_last_sexp::define(vm)?;
    let bindings = bind_journey_keys(vm, false)?;
    let started = Started { launch: Launch::Workspace, loadup: None, bindings, gnu_mode_line: false };
    workspace(vm, &started, files.is_empty())?;
    visit_files(vm, files)?;
    load_gui_extensions(vm, &lisp_dir())?;
    Ok(started)
}

/// `--loadup` and `--gnu-lisp`: run GNU's loadup.el. `font` is the
/// launch font's native `defface default` source. GNU Custom and face files
/// are excluded by the loadup driver so native declarations remain authoritative.
pub fn initialize_loadup(vm: &mut Vm, launch: Launch, font: Option<&str>) -> Result<Started, String> {
    initialize_loadup_from(vm, launch, font, &lisp_dir())
}

/// [`initialize_loadup`] with GNU's Lisp from `dir`.
pub fn initialize_loadup_from(vm: &mut Vm, launch: Launch, font: Option<&str>, dir: &std::path::Path) -> Result<Started, String> {
    // One canonical directory for loadup and for finding rlisp/ beside
    // it: `.` or a symlinked gnu-lisp/ must not pick a different sibling.
    let files = take_queued();
    let dir = &dir.canonicalize().map_err(|e| format!("GNU lisp directory {}: {e}", dir.display()))?;
    // Plain functions only, defined with VM2's own `defun` before GNU's
    // byte-run.el replaces it; the host's paint and quit checks read them.
    vm.eval_source(GUI_SUPPORT)?;
    if launch == Launch::Loadup {
        vm.eval_source(include_str!("../../../rlisp/romacs-startup.el"))?;
        vm.eval_source(include_str!("../../../rlisp/romacs-install-command.el"))?;
        crate::eval_last_sexp::define(vm)?;
    }
    if let Some(font) = font { vm.eval_source(font)?; }
    snapshot_gnu_commands(vm)?;
    let loadup = crate::loadup::drive(vm, dir)?;
    // loadup.el and GNU's subdirs.el step (done by the drive) compute GNU's
    // own load-path; romacs' bundled Lisp (rlisp/, beside gnu-lisp/) goes
    // after it, so it stays loadable by name and cannot be found in place of
    // a GNU file on that path.
    let bundled: Vec<String> = runtime_load_path(dir)?.into_iter().filter(|p| !p.starts_with(dir))
        .map(|p| lisp_string(&p.to_string_lossy())).collect();
    if !bundled.is_empty() {
        vm.eval_source(&format!("(setq load-path (append load-path (list {})))", bundled.join(" ")))?;
    }
    let gnu_mode_line = vm.eval_source("(default-boundp 'mode-line-format)")? != romacs_vm::Fixed::NIL;
    if launch == Launch::GnuOnly {
        if !files.is_empty() {
            return Err("FILE arguments need romacs's visiting command, which --gnu-lisp does not install".into());
        }
        let started = Started { launch, loadup: Some(loadup), bindings: vec![], gnu_mode_line };
        vm.eval_source(&format!(
            "(progn (set-buffer (get-buffer-create \"*scratch*\")) (erase-buffer)
                    (insert {} \"\\n\\n\" {} \"\\n\\nNo romacs stand-ins (--gnu-lisp). Close the window to quit.\\n\\n\")
                    (set-window-buffer (selected-window) (current-buffer)) (goto-char (point-max))
                    (set-window-point (selected-window) (point)))",
            lisp_string(&started.status()), lisp_string(&mode_line_note(gnu_mode_line)),
        ))?;
        return Ok(started);
    }
    vm.eval_source(include_str!("../../../rlisp/romacs-find-file.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-buffer-switch.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-extended-command.el"))?;
    initialize_editing(vm)?;
    vm.eval_source(include_str!("../../../rlisp/romacs-dired.el"))?;
    vm.eval_source(include_str!("../../../rlisp/romacs-term.el"))?;
    let bindings = bind_journey_keys(vm, true)?;
    let started = Started { launch, loadup: Some(loadup), bindings, gnu_mode_line };
    workspace(vm, &started, files.is_empty())?;
    visit_files(vm, &files)?;
    load_gui_extensions(vm, dir)?;
    Ok(started)
}

fn initialize_editing(vm: &mut Vm) -> Result<(), String> {
    vm.eval_source(include_str!("../../../rlisp/romacs-editing.el"))?;
    for (source, names) in [
        (include_str!("../../../gnu-lisp/subr.el"), &[
            "plusp", "add-to-history",
        ][..]),
        (include_str!("../../../gnu-lisp/simple.el"), &[
            "mark", "set-mark", "push-mark", "pop-mark", "push-mark-command",
            "pop-to-mark-command", "set-mark-command", "beginning-of-buffer",
        ][..]),
    ] {
        let (spans, _) = crate::loadup::load_sites(source)?;
        for name in names {
            if vm.eval_source(&format!("(fboundp '{name})"))? != romacs_vm::Fixed::NIL { continue; }
            let prefix = format!("(defun {name}");
            let definitions: Vec<_> = spans.iter().filter(|&&(start, end, _)| {
                source[start..end].strip_prefix(&prefix)
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
            }).collect();
            if definitions.len() != 1 { return Err(format!("Expected one GNU editing definition: {name}")); }
            let &&(start, end, _) = definitions.first().unwrap();
            vm.eval_source(&source[start..end])?;
        }
    }
    Ok(())
}

/// Evaluate sorted `.el` command modules beside GNU's runtime Lisp directory.
/// Both checkouts and installed `share/romacs` use
/// `rlisp/romacs-gui-extensions`; a missing directory is optional.
/// Modules call `romacs-gui-install-command` with a key and quoted definition.
/// Errors abort startup with the module path; GNU-only startup does not load them.
pub fn load_gui_extensions(vm: &mut Vm, dir: &std::path::Path) -> Result<(), String> {
    let dir = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let root = dir.parent().ok_or("Lisp directory has no parent")?
        .join("rlisp/romacs-gui-extensions");
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("{}: {error}", root.display())),
    };
    let mut modules = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", root.display()))?.path();
        if path.extension().is_some_and(|extension| extension == "el") && path.is_file() {
            modules.push(path);
        }
    }
    modules.sort();
    for path in modules {
        let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        vm.eval_source(&source).map_err(|e| format!("GUI command module {}: {e}", path.display()))?;
    }
    Ok(())
}

fn mode_line_note(gnu: bool) -> String {
    if gnu { "Mode line: GNU's mode-line-format.".into() } else {
        "Mode line: mode-line-format is void after loadup (bindings.el did not reach it), so --gnu-lisp draws no mode line and no cursor (layout draws the cursor only in windows that have a mode line).".into()
    }
}

/// The mode line used when `mode-line-format` has no default binding after
/// loadup: the default launch loads no GNU Lisp, and under `--loadup`
/// bindings.el can stop before its default (see DIVERGENCES.md GUI-ML1). Built from GNU's own
/// %-constructs in GNU's order (coding/EOL, modified, remote, buffer name,
/// position, line) and expanded by GNU's display_mode_element/decode_mode_spec;
/// the label says on screen that bindings.el did not load. [`workspace`] gives
/// each of its buffers its own copy with that buffer's tag appended.
pub const STAND_IN_MODE_LINE: &str =
    r#"'("-" "%z%Z%*%+%@" "  " "%12b" "   " "%p L%l" "   " "[stand-in: no bindings.el]")"#;

/// The Rum workspace: *scratch* with a status that names, per key, whether
/// GNU or romacs is running it. With SHOWCASE (no FILE arguments) it sits
/// beside a buffer of VM2 evaluations; without, *scratch* fills the frame and
/// no evaluations buffer exists, so a visited file gets the whole window as
/// in GNU's `emacs FILE`.
fn workspace(vm: &mut Vm, started: &Started, showcase: bool) -> Result<(), String> {
    vm.eval_source(&format!("(setq romacs-gui-preserve-mode-line {})", if started.gnu_mode_line { "t" } else { "nil" }))?;
    let mut by_gnu: Vec<String> = vec![];
    let mut stand_in: Vec<String> = vec![];
    for b in &started.bindings {
        let entry = format!("{} {}", key_label(b.key), b.gnu);
        if b.gnu_defined { by_gnu.push(entry) } else { stand_in.push(entry) }
    }
    by_gnu.dedup();
    stand_in.dedup();
    let list = |items: &[String]| if items.is_empty() { "  (none)\n".to_owned() } else {
        items.iter().map(|i| format!("  {i}\n")).collect::<String>()
    };
    let help = format!(
        "\nType below; multibyte text works.\n\n",
    );
    let mode_line = if started.gnu_mode_line { "nil" } else { "t" };
    vm.eval_source(&format!(
        r#"(progn
  (set-buffer (get-buffer-create "*scratch*"))
  (erase-buffer)
  (romacs-gui-line "romacs / VM" 'font-lock-function-name-face)
  (romacs-gui-line {status} 'romacs-comment)
  (insert {help})
  (romacs-gui-line "GNU COMMANDS (C in GNU, Rust here)" 'font-lock-function-name-face)
  (insert "C-f C-b  char      C-a C-e  line      C-d  delete\n\n")
  (romacs-gui-line "GNU'S LISP COMMANDS, LOADED BY LOADUP" 'font-lock-function-name-face)
  (insert {by_gnu} "\n")
  (romacs-gui-line "ROMACS STAND-INS (GNU'S COMMAND DID NOT LOAD)" 'font-lock-function-name-face)
  (insert {stand_in} "  DEL  delete back (VM2's delete-backward-char)\n\n")
  (romacs-gui-line "FILES AND BUFFERS" 'font-lock-function-name-face)
  (insert "C-x C-f  find local file       C-x b  switch buffer\n")
  (insert "M-x  run interactive command by name     C-x d  browse directory\n")
  (insert "TAB completes; M-p/M-n history; C-g cancels. Local UTF-8 paths.\n")
  (insert "Dired: n/p move, RET open, ^ parent, g refresh, q return.\n\n")
  (romacs-gui-line "NOT YET" 'font-lock-function-name-face)
  (insert "M-< M->: need simple.el.\n")
  (insert "Errors show GNU's message below; the session survives.\n\n")
  (setq romacs-gui-scratch-start (point))
  (when {mode_line}
    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '("   Rum / VM")))
    (make-local-variable 'mode-line-format)
    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '("   Rum / VM"))))
  (set-window-buffer nil (current-buffer))
  (setq romacs-gui-welcome-window (selected-window))
  (set-window-point romacs-gui-welcome-window romacs-gui-scratch-start))"#,
        status = lisp_string(&started.status()),
        help = lisp_string(&help),
        by_gnu = lisp_string(&list(&by_gnu)),
        stand_in = lisp_string(&list(&stand_in)),
    ).replace("@STAND_IN_MODE_LINE@", STAND_IN_MODE_LINE))?;
    if !showcase {
        return Ok(());
    }
    vm.eval_source(&format!(
        r#"(progn
  (setq romacs-gui-results-window (romacs-gui-split t))
  (set-buffer (get-buffer-create "*VM evaluations*"))
  (erase-buffer)
  (when {mode_line}
    (make-local-variable 'mode-line-format)
    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '("   evaluated at startup"))))
  (romacs-gui-line "Evaluated by VM" 'font-lock-function-name-face)
  (insert "\n")
  (set-window-buffer romacs-gui-results-window (current-buffer)))"#,
    ).replace("@STAND_IN_MODE_LINE@", STAND_IN_MODE_LINE))?;
    for source in [
        "(+ 20 22)",
        "(let ((n 7)) (list (* n n) (+ n 1) (- n 1)))",
        "(substring \"Hello, κόσμε!\" 0)",
        "(progn (defun romacs-demo-sum (xs) (if xs (+ (car xs) (romacs-demo-sum (cdr xs))) 0)) (romacs-demo-sum '(3 5 8 13)))",
    ] {
        // Convert immediately to Rust-owned text. No unrooted Value survives an
        // allocation or another evaluation, including under a small GC floor.
        // The default launch aborts on a failing showcase form, as before
        // this lane; after loadup a form may reach GNU code the VM cannot
        // run yet, and the window shows the error instead.
        let printed = match vm.eval_source(source) {
            Ok(result) => format!("=> {}", result.display()?),
            Err(error) if started.launch != Launch::Workspace => format!("=> error: {error}"),
            Err(error) => return Err(format!("showcase evaluation {source}: {error}")),
        };
        let label = if source.starts_with("(progn") {
            "(romacs-demo-sum '(3 5 8 13))"
        } else {
            source
        };
        vm.eval_source(&format!(
            "(progn (romacs-gui-line {} 'romacs-code) (romacs-gui-line {} 'romacs-result) (insert \"\\n\"))",
            lisp_string(label), lisp_string(&printed),
        ))?;
    }
    let stats = vm.heap_stats();
    let sample = format!(
        "Heap sample after evaluation\n  {} live objects / {} bytes\n  {} allocations / {} collections\n",
        stats.live_objects, stats.live_bytes, stats.allocations, stats.collections,
    );
    vm.eval_source(&format!(
        "(progn (romacs-gui-line {} 'romacs-comment) (goto-char 1) (set-window-point romacs-gui-results-window 1) (select-window romacs-gui-welcome-window) (set-buffer (window-buffer (selected-window))) (goto-char romacs-gui-scratch-start) (set-window-point romacs-gui-welcome-window romacs-gui-scratch-start))",
        lisp_string(&sample),
    ))?;
    Ok(())
}

fn key_label(key: &str) -> String {
    let inner = key.trim_matches(|c| c == '[' || c == ']');
    inner.split(' ').map(|k| match k.parse::<u32>() {
        Ok(n) if n < 32 => format!("C-{}", char::from_u32(n + 96).unwrap_or('?')),
        Ok(n) => char::from_u32(n).map(String::from).unwrap_or_default(),
        Err(_) => format!("<{k}>"),
    }).collect::<Vec<_>>().join(" ")
}

fn lisp_string(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_vm::CommandEvent;

    fn printed(vm: &mut Vm, source: &str) -> String {
        let value = vm
            .eval_source(source)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        value.display().unwrap()
    }
    /// Buffer `name`'s own `mode-line-format`, or `GLOBAL` when it follows
    /// the default value (VM2 has no `local-variable-p`: move the default
    /// and see whether the buffer's value follows it).
    fn own_mode_line(name: &str) -> String {
        format!("(with-current-buffer {} (let ((old (default-value 'mode-line-format))) \
                 (set-default 'mode-line-format 'romacs-test-probe) \
                 (prog1 (if (eq mode-line-format 'romacs-test-probe) 'GLOBAL mode-line-format) \
                   (set-default 'mode-line-format old))))", lisp_string(name))
    }
    fn workspace(floor: usize) -> Vm {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        vm.set_collection_floor(floor);
        initialize(&mut vm).unwrap();
        vm
    }
    fn key(vm: &mut Vm, code: i64) {
        let outcome = vm.command_event(CommandEvent::Character(code)).unwrap();
        assert!(outcome.signal.is_none(), "{outcome:?}");
    }
    fn chord(vm: &mut Vm, code: i64) {
        // Test the installed complete key sequence. Native prefix accumulation
        // belongs to the host/command loop, not this startup module.
        printed(
            vm,
            &format!("(call-interactively (key-binding [24 {code}]))"),
        );
    }

    fn fileio_fixture(case: &str) -> (Vm, std::path::PathBuf) {
        let root = std::env::temp_dir()
            .join("romacs-fileio-gui-evidence").join(format!("{case}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let file = root.join("notes.org");
        std::fs::write(&file, "first\nsecond\nthird\n").unwrap();
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        vm.set_collection_floor(1);
        initialize_files(&mut vm, &[file.clone()]).unwrap();
        assert_eq!(printed(&mut vm, "buffer-file-name"), lisp_string(file.to_str().unwrap()), "filename must not be ignored");
        assert_eq!(printed(&mut vm, "(buffer-string)"), "\"first\\nsecond\\nthird\\n\"");
        if let Ok(mutant) = std::env::var("ROMACS_FILEIO_GUI_MUTANT") {
            let source = match mutant.as_str() {
                "supersession" => "(defun verify-visited-file-modtime (&optional buffer) t)",
                "identity" => "(setq romacs-gui-visits nil)",
                "quit" => "(defun romacs-gui-quit () (interactive) (setq romacs-gui-quit-requested t))",
                "indicator" => "(set-default 'mode-line-format '(\"  %b\"))",
                "kill-retain" => "(defun romacs-gui-kill-confirmed (buffer) nil)",
                "kill-wrong-target" => "(defun romacs-gui-kill-confirmed (buffer) (kill-buffer (current-buffer)) (romacs-gui-live-visits))",
                "prompt" => "(defun romacs-gui-echo (text) nil)",
                _ => panic!("unknown GUI mutant {mutant}"),
            };
            printed(&mut vm, source);
        }
        (vm, file)
    }

    fn fileio_native_keys(vm: &mut Vm, keys: &[i64]) {
        let mut commands = crate::command::Commands::default();
        let mut presentation = crate::presentation::Presentation::new();
        for code in keys {
            let outcome = presentation.command(vm, &mut commands, CommandEvent::Character(*code)).unwrap();
            if let Some(outcome) = outcome { println!("native key={code}: {outcome:?}"); }
        }
    }

    #[test]
    fn fileio_gui_kill_confirmed_y_removes_buffer() {
        let (mut vm, file) = fileio_fixture("kill-confirmed");
        printed(&mut vm, "(setq fileio-kill-victim (current-buffer) fileio-kill-name \"notes.org\")");
        fileio_native_keys(&mut vm, &[120, 24, 107]);
        assert_ne!(printed(&mut vm, "romacs-gui-pending"), "nil");
        fileio_native_keys(&mut vm, &[121]);
        assert_eq!(printed(&mut vm, "(eq fileio-kill-victim (get-buffer-create fileio-kill-name))"), "nil", "confirmed y retained victim in the native buffer registry");
        assert_eq!(printed(&mut vm, "(memq fileio-kill-victim romacs-gui-visits)"), "nil");
        assert_eq!(printed(&mut vm, "romacs-gui-pending"), "nil");
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "first\nsecond\nthird\n");
    }

    #[test]
    fn fileio_gui_kill_n_and_quit_retain_buffer() {
        for answer in [110, 7] {
            let (mut vm, _) = fileio_fixture(&format!("kill-cancel-{answer}"));
            printed(&mut vm, "(setq fileio-kill-victim (current-buffer) fileio-kill-name \"notes.org\")");
            fileio_native_keys(&mut vm, &[120, 24, 107, answer]);
            assert_eq!(printed(&mut vm, "(eq fileio-kill-victim (get-buffer-create fileio-kill-name))"), "t");
            assert_ne!(printed(&mut vm, "(memq fileio-kill-victim romacs-gui-visits)"), "nil");
            assert_eq!(printed(&mut vm, "romacs-gui-pending"), "nil");
            let text = printed(&mut vm, "(buffer-string)");
            fileio_native_keys(&mut vm, &[121]);
            assert_ne!(printed(&mut vm, "(buffer-string)"), text);
        }
    }

    #[test]
    fn fileio_gui_kill_query_veto_retains_visit() {
        let (mut vm, _) = fileio_fixture("kill-veto");
        printed(&mut vm, "(setq fileio-kill-victim (current-buffer) fileio-kill-queries 0 kill-buffer-query-functions '(fileio-kill-veto)) (defun fileio-kill-veto () (setq fileio-kill-queries (+ fileio-kill-queries 1)) nil)");
        fileio_native_keys(&mut vm, &[120, 24, 107, 121]);
        assert_eq!(printed(&mut vm, "fileio-kill-queries"), "1");
        assert_eq!(printed(&mut vm, "(buffer-live-p fileio-kill-victim)"), "t");
        assert_ne!(printed(&mut vm, "(memq fileio-kill-victim romacs-gui-visits)"), "nil");
        assert_eq!(printed(&mut vm, "romacs-gui-pending"), "nil");
        assert!(printed(&mut vm, "(buffer-string)").contains('x'));
    }

    #[test]
    fn fileio_gui_kill_confirmed_helper_uses_captured_target() {
        let (mut vm, _) = fileio_fixture("kill-target");
        printed(&mut vm, "(setq fileio-kill-victim (current-buffer)) (set-buffer (get-buffer-create \"kill-bystander\")) (set-window-buffer (selected-window) (current-buffer)) (setq fileio-kill-bystander (current-buffer)) (romacs-gui-kill-confirmed fileio-kill-victim)");
        assert_eq!(printed(&mut vm, "(buffer-live-p fileio-kill-victim)"), "nil", "confirmed helper killed current buffer instead of captured victim");
        assert_eq!(printed(&mut vm, "(buffer-live-p fileio-kill-bystander)"), "t");
        assert_eq!(printed(&mut vm, "(memq fileio-kill-victim romacs-gui-visits)"), "nil");
    }

    #[test]
    fn fileio_gui_kill_clean_and_unvisited_buffers() {
        for unvisited in [false, true] {
            let (mut vm, _) = fileio_fixture(&format!("kill-clean-{unvisited}"));
            if unvisited {
                printed(&mut vm, "(set-buffer (get-buffer-create \"kill-unvisited\")) (set-window-buffer (selected-window) (current-buffer)) (insert \"unvisited text\")");
            }
            printed(&mut vm, "(setq fileio-kill-victim (current-buffer))");
            fileio_native_keys(&mut vm, &[24, 107]);
            assert_eq!(printed(&mut vm, "(buffer-live-p fileio-kill-victim)"), "nil");
            assert_eq!(printed(&mut vm, "romacs-gui-pending"), "nil");
        }
    }

    #[test]
    fn fileio_gui_kill_external_death_does_not_leave_stale_visits() {
        let (mut vm, file) = fileio_fixture("kill-external-death");
        printed(&mut vm, "(setq fileio-kill-victim (current-buffer)) (insert \"unsaved\") (romacs-gui-confirm 'kill) (kill-buffer fileio-kill-victim) (romacs-gui-answer-yes) (romacs-gui-quit)");
        assert_eq!(printed(&mut vm, "romacs-gui-pending"), "nil");
        assert_eq!(printed(&mut vm, "(memq fileio-kill-victim romacs-gui-visits)"), "nil");
        visit_files(&mut vm, &[file]).unwrap();
        assert_eq!(printed(&mut vm, "(eq fileio-kill-victim (current-buffer))"), "nil");
        assert_eq!(printed(&mut vm, "(buffer-string)"), "\"first\\nsecond\\nthird\\n\"");
    }

    #[test]
    fn fileio_gui_e2e_three_before_after_loss_repros() {
        for before in [true, false] {
            let phase = if before { "BEFORE" } else { "AFTER" };
            for case in ["external", "duplicate", "quit"] {
                let (mut vm, file) = fileio_fixture(&format!("e2e-{phase}-{case}"));
                if before { printed(&mut vm, include_str!("../evidence/fileio-gui-before.el")); }
                fileio_native_keys(&mut vm, &[120]);
                match case {
                    "external" => {
                        std::fs::write(&file, "external writer\n").unwrap();
                        fileio_native_keys(&mut vm, &[24, 19]);
                        let disk = std::fs::read_to_string(&file).unwrap();
                        assert_eq!(disk == "external writer\n", !before);
                        println!("{phase} E2E external overwrite: disk={disk:?}");
                    }
                    "duplicate" => {
                        visit_files(&mut vm, &[file.parent().unwrap().join("./notes.org")]).unwrap();
                        fileio_native_keys(&mut vm, &[24, 19]);
                        let disk = std::fs::read_to_string(&file).unwrap();
                        assert_eq!(disk.starts_with('x'), !before);
                        println!("{phase} E2E duplicate alias save: disk={disk:?}");
                    }
                    "quit" => {
                        fileio_native_keys(&mut vm, &[24, 3]);
                        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), if before { "t" } else { "nil" });
                        if !before {
                            fileio_native_keys(&mut vm, &[110]);
                            assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
                            assert!(printed(&mut vm, "(buffer-string)").contains('x'));
                        }
                        println!("{phase} E2E unsaved quit: shutdown permitted={before}");
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn fileio_gui_native_close_prompts_and_keeps_pending_redraw() {
        let (mut vm, file) = fileio_fixture("native-close");
        printed(&mut vm, "(insert \"unsaved\")");
        let mut presentation = crate::presentation::Presentation::new();
        presentation.submitted(1, std::time::Instant::now());
        assert!(!presentation.window_close(&mut vm).unwrap(), "native WindowClose bypassed unsaved confirmation");
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        let prompt = printed(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))");
        assert!(prompt.contains("(y/n"), "native close must show the keyboard quit prompt: {prompt}");
        assert!(!presentation.ready(), "must not overwrite an in-flight frame");
        presentation.completed(1);
        assert!(presentation.ready(), "pending native-close prompt must redraw after the in-flight frame");
        fileio_native_keys(&mut vm, &[110]);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        assert!(!presentation.window_close(&mut vm).unwrap());
        fileio_native_keys(&mut vm, &[7]);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        assert!(!presentation.window_close(&mut vm).unwrap());
        fileio_native_keys(&mut vm, &[121]);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "t");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "first\nsecond\nthird\n");
    }

    #[test]
    fn fileio_gui_quit_rejects_stale_consent_after_other_input() {
        assert_no_stale_consent(false);
    }

    #[test]
    fn fileio_gui_native_close_rejects_stale_consent_after_other_input() {
        assert_no_stale_consent(true);
    }

    fn assert_no_stale_consent(native_close: bool) {
        {
            for interleave in ["edit", "hidden", "switch", "save"] {
                let (mut vm, file) = fileio_fixture(&format!("stale-{native_close}-{interleave}"));
                printed(&mut vm, "(setq fileio-original (current-buffer)) (insert \"unsaved\")");
                if interleave == "hidden" {
                    printed(&mut vm, "(set-buffer (get-buffer-create \"*scratch*\")) (set-window-buffer (selected-window) (current-buffer))");
                }
                let mut presentation = crate::presentation::Presentation::new();
                let mut commands = crate::command::Commands::default();
                if native_close {
                    assert!(!presentation.window_close(&mut vm).unwrap());
                } else {
                    for code in [24, 3] {
                        presentation.command(&mut vm, &mut commands, CommandEvent::Character(code)).unwrap();
                    }
                }
                assert_ne!(printed(&mut vm, "romacs-gui-pending"), "nil");
                if interleave == "switch" {
                    printed(&mut vm, "(set-buffer (get-buffer-create \"*scratch*\")) (set-window-buffer (selected-window) (current-buffer))");
                }
                let before_keys = printed(&mut vm, "(buffer-string)");
                let keys: &[i64] = if interleave == "save" { &[24, 19, 121] }
                    else if interleave == "switch" { &[121] } else { &[97, 121] };
                for code in keys {
                    presentation.command(&mut vm, &mut commands, CommandEvent::Character(*code)).unwrap();
                }
                assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil", "late y consent: native_close={native_close}, interleave={interleave}");
                assert_eq!(printed(&mut vm, "romacs-gui-pending"), "nil");
                let original = printed(&mut vm, "(with-current-buffer fileio-original (buffer-string))");
                assert!(original.contains("unsaved"));
                if interleave == "edit" {
                    assert_eq!(original, "\"unsavedayfirst\\nsecond\\nthird\\n\"");
                } else if interleave == "hidden" {
                    assert_eq!(printed(&mut vm, "(buffer-string)"), format!("{}ay\"", before_keys.strip_suffix('"').unwrap()));
                    assert_eq!(original, "\"unsavedfirst\\nsecond\\nthird\\n\"");
                }
                if interleave != "save" {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first\nsecond\nthird\n");
                } else {
                    assert!(std::fs::read_to_string(&file).unwrap().contains("unsaved"));
                }
                assert!(printed(&mut vm, "(buffer-string)").contains('y'), "late y must insert normally");
                println!("stale consent rejected: native_close={native_close}, interleave={interleave}");
            }
        }
    }

    #[test]
    fn fileio_gui_quit_checks_hidden_visit_and_restores_answer_keys() {
        let (mut vm, _) = fileio_fixture("hidden");
        printed(&mut vm, "(insert \"unsaved\") (set-buffer (get-buffer-create \"*scratch*\")) (set-window-buffer (selected-window) (current-buffer))");
        fileio_native_keys(&mut vm, &[24, 3, 110]);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        let before = printed(&mut vm, "(buffer-string)");
        fileio_native_keys(&mut vm, &[121]);
        assert_ne!(printed(&mut vm, "(buffer-string)"), before, "answer y must return to ordinary insertion after n in a non-visiting buffer");
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
    }

    #[test]
    fn fileio_modtime_command_save_prompts_and_skips_content_reads() {
        use std::os::unix::fs::MetadataExt;
        let (mut vm, file) = fileio_fixture("mtime-command");
        let meta = std::fs::metadata(&file).unwrap();
        let original = std::fs::read(&file).unwrap();
        printed(&mut vm, "(insert \"local\")");
        // A same-size, unchanged-modtime external write is deliberately not
        // detected by GNU. The production C-x C-s route must never read it.
        std::fs::write(&file, vec![b'z'; original.len()]).unwrap();
        let stamp = std::time::UNIX_EPOCH + std::time::Duration::new(meta.mtime() as u64, meta.mtime_nsec() as u32);
        std::fs::File::open(&file).unwrap().set_times(std::fs::FileTimes::new().set_modified(stamp)).unwrap();
        printed(&mut vm, "(fset 'insert-file-contents (lambda (&rest args) (signal 'error '(\"CONTENT_READ_FORBIDDEN\")))) nil");
        fileio_native_keys(&mut vm, &[24, 19]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "localfirst\nsecond\nthird\n", "MTIME_COMMAND_NO_CONTENT_READ");
        printed(&mut vm, "(insert \"again\")");
        std::fs::write(&file, "external writer\n").unwrap();
        fileio_native_keys(&mut vm, &[24, 19]);
        assert_eq!(printed(&mut vm, "(car romacs-gui-pending)"), "save", "MTIME_SUPERSESSION_PROMPT");
        assert_eq!(printed(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))"), "\"File changed on disk; overwrite it? (y/n; C-g cancels)\"", "MTIME_TARGET_MESSAGE");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "external writer\n");
        fileio_native_keys(&mut vm, &[110]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "external writer\n", "MTIME_CANCEL_PRESERVES_DISK");
        fileio_native_keys(&mut vm, &[24, 19, 121]);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "localagainfirst\nsecond\nthird\n", "MTIME_CONFIRMED_SAVE");
        assert_eq!(printed(&mut vm, "(verify-visited-file-modtime)"), "t");
        assert_eq!(printed(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))"), "\"Saved\"", "MTIME_CONFIRMED_NOT_CANCELLED");
        println!("C-x C-s saved unchanged metadata with CONTENT_READ_FORBIDDEN; supersession n/y through native command dispatch");
    }

    #[test]
    fn fileio_gui_supersession_preserves_external_bytes() {
        for modified in [false, true] {
            let (mut vm, file) = fileio_fixture(if modified { "supersession-dirty" } else { "supersession-clean" });
            if modified { printed(&mut vm, "(insert \"local edit\")"); }
            std::fs::write(&file, "external edit\n").unwrap();
            let before = printed(&mut vm, "(buffer-string)");
            let outcome = vm.eval_source("(romacs-gui-save-buffer)").map(|_| ()).map_err(|e| e.to_string());
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "external edit\n", "save overwrote an external writer (modified={modified}, outcome={outcome:?})");
            assert_eq!(printed(&mut vm, "(buffer-string)"), before);
            if modified { assert_eq!(printed(&mut vm, "(car romacs-gui-pending)"), "save", "dirty supersession must prompt"); }
            println!("supersession modified={modified}: disk external edit preserved, buffer preserved");
        }
    }

    #[test]
    fn fileio_gui_supersession_fails_closed_on_unreadable_text_or_removed_file() {
        for binary in [false, true] {
            let (mut vm, file) = fileio_fixture(if binary { "binary-external" } else { "removed-external" });
            printed(&mut vm, "(insert \"local edit\")");
            let original = printed(&mut vm, "(buffer-string)");
            if binary { std::fs::write(&file, [255, 0, 128]).unwrap(); }
            else { std::fs::remove_file(&file).unwrap(); }
            vm.eval_source("(romacs-gui-save-buffer)").unwrap();
            assert_eq!(printed(&mut vm, "(car romacs-gui-pending)"), "save");
            assert_eq!(printed(&mut vm, "(buffer-string)"), original);
            if binary { assert_eq!(std::fs::read(file).unwrap(), vec![255, 0, 128]); }
            else { assert!(!file.exists(), "save must not recreate an externally removed file"); }
        }
    }

    #[test]
    fn file_identity_gui_hardlink_reuses_native_visit() {
        let (mut vm, file) = fileio_fixture("identity-hardlink");
        let hard = file.with_file_name("hard.org");
        let _ = std::fs::remove_file(&hard);
        std::fs::hard_link(&file, &hard).unwrap();
        printed(&mut vm, "(setq identity-original (current-buffer)) (insert \"live\")");
        let sym = file.with_file_name("symlink.org");
        let _ = std::fs::remove_file(&sym);
        std::os::unix::fs::symlink(&file, &sym).unwrap();
        for alias in [hard, sym, file.parent().unwrap().join("./notes.org")] {
            visit_files(&mut vm, &[alias]).unwrap();
            assert_eq!(printed(&mut vm, "(eq identity-original (current-buffer))"), "t", "IDENTITY_GUI_HARDLINK_REUSES_NATIVE_BUFFER");
        }
        assert_eq!(printed(&mut vm, "(eq identity-original (current-buffer))"), "t", "IDENTITY_GUI_HARDLINK_REUSES_NATIVE_BUFFER");
        assert_eq!(printed(&mut vm, "(buffer-string)"), "\"livefirst\\nsecond\\nthird\\n\"", "IDENTITY_GUI_PRESERVES_EDITS");
    }

    #[test]
    fn file_identity_gui_adopts_direct_lisp_visit_and_checks_quit() {
        let (mut vm, file) = fileio_fixture("identity-direct");
        let direct = file.with_file_name("direct.org");
        let hard = file.with_file_name("direct-hard.org");
        std::fs::write(&direct, "direct-disk").unwrap();
        let _ = std::fs::remove_file(&hard);
        std::fs::hard_link(&direct, &hard).unwrap();
        printed(&mut vm, &format!("(setq identity-direct (get-buffer-create (generate-new-buffer-name \"direct\"))) (set-buffer identity-direct) (insert-file-contents {} t) (insert \"live\")", lisp_string(direct.to_str().unwrap())));
        visit_files(&mut vm, &[hard]).unwrap();
        assert_eq!(printed(&mut vm, "(eq identity-direct (current-buffer))"), "t", "IDENTITY_GUI_ADOPTS_DIRECT_LISP_VISIT");
        assert_eq!(printed(&mut vm, "(and (memq identity-direct romacs-gui-visits) (romacs-gui-dirty-p) t)"), "t", "IDENTITY_GUI_ADOPTED_VISIT_REMAINS_DIRTY");
    }

    #[test]
    fn file_identity_gui_unreadable_duplicate_reports_without_aborting() {
        use std::os::unix::fs::PermissionsExt;
        let (mut vm, file) = fileio_fixture("identity-unreadable");
        let denied = file.with_file_name("denied.org");
        let hard = file.with_file_name("denied-hard.org");
        std::fs::write(&denied, "denied").unwrap();
        let _ = std::fs::remove_file(&hard);
        std::fs::hard_link(&denied, &hard).unwrap();
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0)).unwrap();
        assert!(std::fs::read(&denied).is_err(), "IDENTITY_DENIED_FIXTURE_REALLY_UNREADABLE");
        let result = visit_files(&mut vm, &[denied.clone(), denied.clone(), hard]);
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(result.is_ok(), "IDENTITY_GUI_DUPLICATE_PERMISSION_ERROR_MUST_NOT_ABORT: {result:?}");
        assert_eq!(printed(&mut vm, "(length romacs-gui-visit-errors)"), "3", "IDENTITY_GUI_EACH_DENIED_VISIT_RECORDED");
    }

    #[test]
    fn fileio_gui_duplicate_visit_reuses_live_edits() {
        let (mut vm, file) = fileio_fixture("duplicate");
        printed(&mut vm, "(setq fileio-first-buffer (current-buffer)) (insert \"local edit\")");
        let alias = file.parent().unwrap().join("./notes.org");
        visit_files(&mut vm, &[alias]).unwrap();
        assert_eq!(printed(&mut vm, "(eq fileio-first-buffer (current-buffer))"), "t", "second visit created a stale competing buffer");
        assert_eq!(printed(&mut vm, "(buffer-string)"), "\"local editfirst\\nsecond\\nthird\\n\"");
        chord(&mut vm, 19);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "local editfirst\nsecond\nthird\n");
        println!("duplicate alias visit retained same buffer and saved its live edits");
    }

    #[test]
    fn fileio_gui_modified_quit_requires_explicit_confirmation() {
        let (mut vm, file) = fileio_fixture("quit");
        printed(&mut vm, "(insert \"unsaved\")");
        chord(&mut vm, 3);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil", "C-x C-c discarded unsaved work without confirmation");
        let prompt = printed(&mut vm, "(with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))");
        assert!(prompt.contains("(y/n"), "GUI confirmation must be visible, not stdout only: {prompt}");
        key(&mut vm, 110);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        assert!(printed(&mut vm, "(buffer-string)").contains("unsaved"));
        chord(&mut vm, 3);
        key(&mut vm, 7);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        chord(&mut vm, 3);
        key(&mut vm, 121);
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "t");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "first\nsecond\nthird\n");
        println!("quit: initial request, n and C-g preserve editor; y explicitly permits discard");
    }

    #[test]
    fn fileio_gui_modified_kill_requires_explicit_confirmation() {
        let (mut vm, _) = fileio_fixture("kill");
        printed(&mut vm, "(setq fileio-kill-name (current-buffer)) (insert \"unsaved\")");
        chord(&mut vm, 107);
        assert_eq!(printed(&mut vm, "(and (get-buffer fileio-kill-name) t)"), "t", "C-x k discarded unsaved buffer");
        key(&mut vm, 110);
        assert_eq!(printed(&mut vm, "(and (get-buffer fileio-kill-name) t)"), "t");
        chord(&mut vm, 107);
        key(&mut vm, 121);
        assert_eq!(printed(&mut vm, "(and (get-buffer fileio-kill-name) t)"), "t", "get-buffer preserves a retained buffer object's identity, even when dead");
        assert_eq!(printed(&mut vm, "(buffer-live-p fileio-kill-name)"), "nil", "KILL_CONFIRMED_BUFFER_DEAD");
    }

    #[test]
    fn fileio_gui_plus_line_is_not_a_filename() {
        let (mut vm, file) = fileio_fixture("plus-line");
        initialize_files(&mut vm, &["+2".into(), file.clone()]).unwrap();
        assert_eq!(printed(&mut vm, "(point)"), "7");
        initialize_files(&mut vm, &["+2:3".into(), file.clone()]).unwrap();
        assert_eq!(printed(&mut vm, "buffer-file-name"), lisp_string(file.to_str().unwrap()));
        assert_eq!(printed(&mut vm, "(point)"), "9");
        let literal = std::env::current_dir().unwrap().join("+2:3");
        initialize_files(&mut vm, &["--".into(), "+2:3".into()]).unwrap();
        assert_eq!(printed(&mut vm, "buffer-file-name"), lisp_string(literal.to_str().unwrap()));
        assert_eq!(printed(&mut vm, "(buffer-string)"), "\"\"");
        assert_eq!(printed(&mut vm, "(let ((buffers romacs-gui-visits) (found nil)) (while buffers (with-current-buffer (car buffers) (when (and buffer-file-name (string-match \"/+2$\" buffer-file-name)) (setq found t))) (setq buffers (cdr buffers))) found)"), "nil");
    }

    #[test]
    fn fileio_gui_position_columns_follow_display_cells_and_require_digits() {
        let (mut vm, file) = fileio_fixture("display-columns");
        for (text, argument, point) in [("\tabc\n", "+1:4", "2"), ("日本語\n", "+1:3", "2"), ("a\u{301}b\n", "+1:3", "4")] {
            printed(&mut vm, &format!("(erase-buffer) (insert {})", lisp_string(text)));
            visit_files(&mut vm, &[argument.into(), file.clone()]).unwrap();
            assert_eq!(printed(&mut vm, "(point)"), point, "{argument}: {text:?}");
        }
        printed(&mut vm, "(setq romacs-gui-preserve-mode-line t) (set-default 'mode-line-format '(\"GNU sentinel\"))");
        visit_files(&mut vm, &[file.clone()]).unwrap();
        assert_eq!(printed(&mut vm, "(default-value 'mode-line-format)"), "(\"GNU sentinel\")");
        for name in ["++2", "+2:+3", "+2:", "+:3"] {
            assert!(parse_position_number(name).is_none());
            visit_files(&mut vm, &[name.into()]).unwrap();
            assert_eq!(printed(&mut vm, "buffer-file-name"), lisp_string(std::env::current_dir().unwrap().join(name).to_str().unwrap()));
        }
        let mut gnu_only = Vm::new();
        gnu_only.eval_source(GUI_SUPPORT).unwrap();
        assert!(crate::presentation::Presentation::new().window_close(&mut gnu_only).unwrap(), "GNU-only launch must still allow native window close without GUI stand-ins");
    }

    #[test]
    fn fileio_gui_reverse_edit_to_saved_bytes_stays_modified() {
        let (mut vm, file) = fileio_fixture("counter-history");
        let before = vm.frame_snapshot(1200, 800).unwrap().windows[0].snapshot.mode_line.clone().unwrap().utf8;
        let original = printed(&mut vm, "(buffer-string)");
        fileio_native_keys(&mut vm, &[120, 127]);
        assert_eq!(printed(&mut vm, "(buffer-string)"), original, "COUNTERS_GUI_REVERSE_BYTES_EQUAL");
        let dirty = printed(&mut vm, "(romacs-gui-dirty-p)");
        let reversed = vm.frame_snapshot(1200, 800).unwrap().windows[0].snapshot.mode_line.clone().unwrap().utf8;
        println!("COUNTERS_REVERSE_EDIT bytes_equal=true dirty={dirty} before={before:?} reversed={reversed:?}; insertion followed by DEL, not undo");
        assert_eq!(dirty, "t", "COUNTERS_GUI_HISTORY_NOT_TEXT_EQUALITY");
        assert_ne!(before, reversed, "COUNTERS_GUI_REVERSE_INDICATOR_STAYS_MODIFIED");
        fileio_native_keys(&mut vm, &[24, 19]);
        assert_eq!(printed(&mut vm, "(romacs-gui-dirty-p)"), "nil", "COUNTERS_GUI_SAVE_CLEARS_NATIVE_STATE");
        assert_eq!(std::fs::read_to_string(file).unwrap(), "first\nsecond\nthird\n");
    }

    #[test]
    fn fileio_gui_visiting_wires_modified_indicator() {
        let (mut vm, _) = fileio_fixture("indicator");
        let before = vm.frame_snapshot(1200, 800).unwrap().windows[0].snapshot.mode_line.clone().unwrap().utf8;
        key(&mut vm, 120);
        let dirty = vm.frame_snapshot(1200, 800).unwrap().windows[0].snapshot.mode_line.clone().unwrap().utf8;
        chord(&mut vm, 19);
        let saved = vm.frame_snapshot(1200, 800).unwrap().windows[0].snapshot.mode_line.clone().unwrap().utf8;
        assert_ne!(before, dirty, "visited buffer mode line does not expose modification");
        assert_eq!(before, saved);
    }

    #[test]
    fn real_results_and_rum_survive_collection() {
        let mut vm = workspace(1);
        assert!(vm.heap_stats().collections > 0);
        assert_eq!(
            printed(
                &mut vm,
                "(eq (current-buffer) (get-buffer \"*scratch*\"))"
            ),
            "t"
        );
        assert_eq!(vm.frame_snapshot(1200, 800).unwrap().windows.len(), 3);
        let text = printed(
            &mut vm,
            "(with-current-buffer \"*VM evaluations*\" (buffer-string))",
        );
        for result in [
            "=> 42",
            "=> (49 8 6)",
            "Hello, κόσμε!",
            "=> 29",
            "live objects /",
            "allocations /",
        ] {
            assert!(text.contains(result), "missing {result}: {text}");
        }
        assert_eq!(printed(&mut vm, "(romacs-demo-sum '(10 20))"), "30");
        vm.collect_garbage();
        let after = printed(
            &mut vm,
            "(with-current-buffer \"*VM evaluations*\" (buffer-string))",
        );
        assert_eq!(text, after);
        let face = vm.display_default_face().unwrap();
        assert_eq!(face.foreground, [0xd4, 0xc5, 0xa0]);
        assert_eq!(face.background, [0, 0, 0]);
    }

    #[test]
    fn startup_publishes_mode_lines_and_exact_rum_faces() {
        let mut vm = workspace(1);
        let snapshot = vm.frame_snapshot(1200, 800).unwrap();
        for (window, name) in snapshot
            .windows
            .iter()
            .take(2)
            .zip(["*scratch*", "*VM evaluations*"])
        {
            let band = window
                .snapshot
                .mode_line
                .as_ref()
                .expect("visible mode line");
            assert!(band.utf8.contains(name), "{}", band.utf8);
        }
        assert_eq!(
            printed(
                &mut vm,
                "(with-current-buffer \"*VM evaluations*\" mode-line-format)"
            ),
            "(\"-\" \"%z%Z%*%+%@\" \"  \" \"%12b\" \"   \" \"%p L%l\" \"   \" \"[stand-in: no bindings.el]\" \"   evaluated at startup\")"
        );
        // Each workspace buffer has its own format, as before the stand-in.
        for name in ["*scratch*", "*VM evaluations*"] {
            assert_ne!(printed(&mut vm, &own_mode_line(name)), "GLOBAL", "{name}");
        }
        // The format is the labelled stand-in for bindings.el, expanded by
        // GNU's engine: both windows show their own buffer and position.
        for window in &snapshot.windows[..2] {
            let band = &window.snapshot.mode_line.as_ref().unwrap().utf8;
            assert!(band.starts_with("---:"), "{band}");
            assert!(band.contains("[stand-in: no bindings.el]"), "{band}");
            assert!(band.contains(" L"), "{band}");
        }
        assert_eq!(
            printed(&mut vm, "(get-text-property 1 'face)"),
            "font-lock-function-name-face"
        );
        let cases = [
            ("default", "d4c5a0", "000000"),
            ("cursor", "", "9099d9"),
            ("mode-line", "d4c5a0", "50269a"),
            ("mode-line-inactive", "756b59", "141414"),
            ("region", "d4c5a0", "3e4451"),
            ("fringe", "000000", "000000"),
            ("highlight", "d4c5a0", "7030af"),
            ("minibuffer-prompt", "d2b580", ""),
            ("font-lock-keyword-face", "b395db", ""),
            ("font-lock-string-face", "aacb75", ""),
            ("font-lock-comment-face", "756b59", ""),
            ("font-lock-function-name-face", "c28fea", ""),
            ("font-lock-variable-name-face", "00a6a6", ""),
            ("font-lock-constant-face", "72b6e4", ""),
            ("font-lock-type-face", "00b8ad", ""),
            ("font-lock-builtin-face", "d0a3e8", ""),
            ("font-lock-warning-face", "e89e30", ""),
            ("link", "4cc9f0", ""),
            ("error", "c97054", ""),
            ("warning", "e89e30", ""),
            ("success", "00b8ad", ""),
        ];
        for (name, foreground, background) in cases {
            printed(&mut vm, &format!("(put-text-property 1 2 'face '{name})"));
            let snapshot = vm.frame_snapshot(1200, 800).unwrap();
            let window = &snapshot.windows[0].snapshot;
            let face = &window.faces[window.text.face_runs[0].face.0 as usize];
            for (expected, actual) in [(foreground, face.foreground), (background, face.background)]
            {
                if !expected.is_empty() {
                    let rgb: Vec<u8> = (0..6)
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&expected[i..i + 2], 16).unwrap())
                        .collect();
                    assert_eq!(actual.as_slice(), rgb.as_slice(), "{name}");
                }
            }
        }
    }

    #[test]
    fn a_file_argument_shows_only_that_file_in_one_window() {
        let dir = std::env::temp_dir().join(format!("romacs-startup-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notes.org");
        std::fs::write(&file, "* Heading\nλ body\n").unwrap();
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        vm.set_collection_floor(1);
        initialize_files(&mut vm, &[file.clone()]).unwrap();
        // GNU's `emacs FILE` shows the file in the whole frame: no demo split.
        let snapshot = vm.frame_snapshot(1200, 800).unwrap();
        assert_eq!(snapshot.windows.len(), 2, "one window plus the minibuffer");
        let band = snapshot.windows[0].snapshot.mode_line.as_ref().expect("visible mode line");
        assert!(band.utf8.contains("notes.org"), "{}", band.utf8);
        assert_eq!(printed(&mut vm, "(get-buffer \"*VM evaluations*\")"), "nil");
        let path = lisp_string(file.to_str().unwrap());
        assert_eq!(
            printed(&mut vm, &format!(
                "(with-current-buffer (window-buffer (selected-window)) (list (equal buffer-file-name {path}) (buffer-string) (window-point (selected-window))))")),
            "(t \"* Heading\\nλ body\\n\" 1)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_point_is_in_blank_editable_area_after_help() {
        let mut vm = workspace(1);
        assert_eq!(
            printed(
                &mut vm,
                "(and (= (point) (point-max)) (= (point) romacs-gui-scratch-start) (= (window-point romacs-gui-welcome-window) (point)))"
            ),
            "t"
        );
        let help = printed(&mut vm, "(buffer-string)");
        assert!(help.ends_with("survives.\\n\\n\""), "{help}");
        key(&mut vm, 120);
        assert_eq!(
            printed(
                &mut vm,
                "(buffer-substring (point-min) romacs-gui-scratch-start)"
            ),
            help
        );
        assert_eq!(
            printed(
                &mut vm,
                "(buffer-substring romacs-gui-scratch-start (point-max))"
            ),
            "\"x\""
        );
        let snapshot = vm.frame_snapshot(1200, 800).unwrap();
        assert_eq!(
            snapshot.windows[0].snapshot.point_byte,
            snapshot.windows[0].snapshot.text.utf8.len()
        );
    }

    #[test]
    fn keymap_commands_manage_nested_windows_and_quit() {
        let mut vm = workspace(1);
        chord(&mut vm, 50); // C-x 2
        chord(&mut vm, 51); // C-x 3
        assert_eq!(
            printed(&mut vm, "(length (romacs-gui-leaves (romacs-gui-root)))"),
            "4"
        );
        chord(&mut vm, 111); // C-x o
        chord(&mut vm, 48); // C-x 0
        assert_eq!(
            printed(&mut vm, "(length (romacs-gui-leaves (romacs-gui-root)))"),
            "3"
        );
        chord(&mut vm, 49); // C-x 1
        assert_eq!(
            printed(&mut vm, "(length (romacs-gui-leaves (romacs-gui-root)))"),
            "1"
        );
        key(&mut vm, 120);
        assert_eq!(
            printed(&mut vm, "(buffer-substring (- (point) 1) (point))"),
            "\"x\""
        );
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "nil");
        chord(&mut vm, 3); // C-x C-c
        assert_eq!(printed(&mut vm, "romacs-gui-quit-requested"), "t");
    }

    /// GNU's `loadup.el` preload order for the WARM census: loadup's first 17
    /// load sites.
    const GNU_BOOTSTRAP_CHAIN: [&str; 17] = [
        "emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote", "subr",
        "keymap", "version", "widget", "custom", "emacs-lisp/map-ynp", "international/mule",
        "international/mule-conf", "env", "format", "bindings", "window", "files",
        "emacs-lisp/macroexp",
    ];

    #[test]
    fn preload_chain_is_the_warm_census_chain() {
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/lisp-census.py")).unwrap();
        let start = script.find("BOOTSTRAP_CHAIN = [").expect("census chain") + "BOOTSTRAP_CHAIN = [".len();
        let list = &script[start..start + script[start..].find(']').unwrap()];
        let census: Vec<&str> = list.split(',').map(|s| s.trim().trim_matches('\'')).filter(|s| !s.is_empty()).collect();
        assert_eq!(census, GNU_BOOTSTRAP_CHAIN.to_vec());
        // ... and it is loadup.el's own first 17 load sites, which the GUI runs.
        let loadup = std::fs::read_to_string(lisp_dir().join("loadup.el")).unwrap();
        let (_, sites) = crate::loadup::load_sites(&loadup).unwrap();
        let first: Vec<&str> = sites.iter().take(17).map(|s| s.name.as_str()).collect();
        assert_eq!(first, GNU_BOOTSTRAP_CHAIN.to_vec());
    }

    /// Run `f` on a thread with the GUI evaluator's 64 MiB stack.
    fn on_evaluator<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap()
    }

    fn scratch_lisp(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("romacs-startup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files { std::fs::write(dir.join(file), text).unwrap(); }
        dir
    }

    #[test]
    fn installed_runtime_root_selects_its_gnu_lisp() {
        // The Nix wrapper sets only ROMACS_RUNTIME_ROOT=$out/share/romacs. The
        // compile-time checkout path does not exist on an installed system.
        let root = std::ffi::OsString::from("/nix/store/x-romacs/share/romacs");
        assert_eq!(lisp_dir_from(None, Some(root.clone())), std::path::Path::new("/nix/store/x-romacs/share/romacs/gnu-lisp"));
        // The more specific override still wins.
        assert_eq!(lisp_dir_from(Some("/elsewhere/lisp".into()), Some(root)), std::path::Path::new("/elsewhere/lisp"));
        assert_eq!(lisp_dir_from(None, None), std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../gnu-lisp"));
    }

    #[test]
    fn runtime_load_path_finds_dependencies_without_exporting_vm_helpers() {
        let dir = lisp_dir().canonicalize().unwrap();
        let owned = dir.parent().unwrap().join("rlisp");
        let path = runtime_load_path(&dir).unwrap();
        let mut vm = Vm::new();
        vm.set_load_path(&path).unwrap();
        for name in ["websocket", "fsm", "markdown-mode"] {
            let directory = owned.join("deps").join(name);
            assert!(path.contains(&directory), "RLISP_DEPENDENCY_NOT_LOADABLE: {name}");
            let expected = directory.join(format!("{name}.el"));
            assert_eq!(printed(&mut vm, &format!("(locate-file-internal {} load-path '(\".el\"))", lisp_string(name))),
                lisp_string(&expected.to_string_lossy()));
        }
        assert!(!path.contains(&owned.join("vm")), "VM helpers are compile-time inputs, not public libraries");
    }

    #[test]
    fn runtime_root_resolves_gnu_lisp_before_bundled_lisp() {
        let root = std::env::var_os("ROMACS_RUNTIME_ROOT").map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .canonicalize().unwrap();
        let dir = lisp_dir_from(None, Some(root.clone().into_os_string())).canonicalize().unwrap();
        let mut vm = Vm::new();
        vm.set_load_path(&runtime_load_path(&dir).unwrap()).unwrap();
        let locate = |vm: &mut Vm, name: &str| printed(vm, &format!("(locate-file-internal {} load-path '(\".elc\" \".el\"))", lisp_string(name)));
        let gnu = root.join("gnu-lisp/obsolete/gnus-dbus.el");
        assert!(gnu.is_file());
        assert_eq!(locate(&mut vm, "gnus-dbus"), lisp_string(&gnu.to_string_lossy()));
        for (name, file) in [("hermes", "hermes/hermes.el"), ("jabber", "jabber/jabber.el"),
                             ("gnosis", "gnosis/gnosis.el"), ("keymap-popup", "emacs-lisp/keymap-popup.el"),
                             ("websocket", "deps/websocket/websocket.el")] {
            assert_eq!(locate(&mut vm, name), lisp_string(&root.join("rlisp").join(file).to_string_lossy()), "{name}");
        }
        assert_eq!(locate(&mut vm, "romacs-win"), "nil");
    }

    #[test]
    fn runtime_load_path_finds_bundled_lisp_beside_gnu_lisp() {
        let dir = lisp_dir().canonicalize().unwrap();
        let path = runtime_load_path(&dir).unwrap();
        let owned = dir.parent().unwrap().join("rlisp");
        let deps = owned.join("deps");
        let first_owned = path.iter().position(|p| p == &owned).expect("rlisp on load-path");
        let first_deps = path.iter().position(|p| p == &deps).expect("rlisp/deps on load-path");
        assert!(path[..first_owned].iter().all(|p| p.starts_with(&dir)), "GNU entries precede owned: {path:?}");
        assert!(path[first_owned..first_deps].iter().all(|p| p.starts_with(&owned) && !p.starts_with(&deps)), "{path:?}");
        assert!(path[first_deps..].iter().all(|p| p.starts_with(&deps)), "{path:?}");
        assert!(owned.join("romacs-mcp-companion.el").is_file());
        for (directory, file) in [("hermes", "hermes.el"), ("jabber", "jabber.el"), ("deps/fsm", "fsm.el")] {
            let entry = owned.join(directory);
            assert!(path.contains(&entry) && entry.join(file).is_file(), "{} not loadable from {path:?}", entry.join(file).display());
        }
        let root = std::env::temp_dir().join(format!("romacs-loadpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gnu-lisp/emacs-lisp")).unwrap();
        assert_eq!(runtime_load_path(&root.join("gnu-lisp")).unwrap(), vec![root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]);
        for directory in ["rlisp/hermes", "rlisp/deps/websocket", "rlisp/vm"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let installed = runtime_load_path(&root.join("gnu-lisp")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(installed, vec![root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp"), root.join("rlisp"),
            root.join("rlisp/hermes"), root.join("rlisp/deps"), root.join("rlisp/deps/websocket")]);
    }

    #[test]
    fn synthetic_name_collision_keeps_gnu_first_before_and_after_loadup() {
        let root = std::env::temp_dir().join(format!("romacs-loadpath-collision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let gnu = root.join("gnu-lisp");
        let owned = root.join("rlisp/example");
        std::fs::create_dir_all(&gnu).unwrap();
        std::fs::create_dir_all(&owned).unwrap();
        std::fs::write(gnu.join("loadup.el"), "").unwrap();
        std::fs::write(gnu.join("collision.el"), "(provide 'collision)").unwrap();
        std::fs::write(owned.join("collision.el"), "(error \"wrong owner\")").unwrap();
        let in_vm = gnu.clone();
        on_evaluator(move || {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
            vm.set_load_path(&runtime_load_path(&in_vm).unwrap()).unwrap();
            let query = "(locate-file-internal \"collision\" load-path '(\".el\"))";
            let expected = lisp_string(&in_vm.join("collision.el").to_string_lossy());
            assert_eq!(printed(&mut vm, query), expected, "GNU_WINS_NAME_COLLISION");
            initialize_loadup_from(&mut vm, Launch::GnuOnly, None, &in_vm).unwrap();
            assert_eq!(printed(&mut vm, query), expected, "GNU_WINS_NAME_COLLISION_AFTER_LOADUP");
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A lisp directory given through a symlink (or as `.`) still finds
    /// the bundled Lisp beside GNU's real lisp/, as master's load path did.
    #[test]
    fn loadup_finds_bundled_lisp_through_a_symlinked_lisp_dir() {
        let real = lisp_dir().canonicalize().unwrap();
        let root = std::env::temp_dir().join(format!("romacs-loadup-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&real, root.join("gnu-lisp")).unwrap();
        let link = root.join("gnu-lisp");
        let hermes = real.parent().unwrap().join("rlisp").join("hermes");
        let found = on_evaluator(move || {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
            initialize_loadup_from(&mut vm, Launch::GnuOnly, None, &link).unwrap();
            let v = vm.eval_source(&format!("(and (member {} load-path) t)", lisp_string(&hermes.to_string_lossy()))).unwrap();
            v.display().unwrap()
        });
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, "t", "bundled Lisp beside the symlink target is not on load-path");
    }

    /// `--gnu-lisp`/`--loadup`: GNU's directories come before every bundled
    /// one, so a bundled file is never found in place of a GNU file of the
    /// same name on GNU's own `load-path`. The synthetic collision test
    /// covers name precedence without retaining an obsolete bundled copy.
    #[test]
    fn loadup_load_path_finds_every_gnu_file_before_bundled_lisp() {
        let dir = lisp_dir().canonicalize().unwrap();
        let bundled = dir.parent().unwrap().join("rlisp");
        let in_vm = dir.clone();
        let (dbus, path, collisions) = on_evaluator(move || {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
            initialize_loadup_from(&mut vm, Launch::GnuOnly, None, &in_vm).unwrap();
            let found = vm.eval_source("(locate-file-internal \"gnus-dbus\" load-path '(\".elc\" \".el\"))")
                .and_then(|v| v.display()).map_err(|e| e.to_string());
            assert_eq!(found,
                Ok(lisp_string(&in_vm.join("obsolete/gnus-dbus.el").to_string_lossy())),
                "locate-file-internal: load-path must resolve GNU before bundled Lisp after loadup");
            let n: usize = printed(&mut vm, "(length load-path)").parse().unwrap();
            let path: Vec<std::path::PathBuf> = (0..n).map(|i| {
                let v = vm.eval_source(&format!("(nth {i} load-path)")).unwrap();
                std::path::PathBuf::from(serde_json::from_str::<String>(&v.display().unwrap()).unwrap())
            }).collect();
            // Every bundled .el whose name a GNU load-path directory also has.
            let gnu: Vec<_> = path.iter().filter(|p| p.starts_with(&in_vm)).cloned().collect();
            let mut collisions = vec![];
            for entry in path.iter().filter(|p| !p.starts_with(&in_vm)) {
                let mut names: Vec<_> = std::fs::read_dir(entry).unwrap().map(|e| e.unwrap().path())
                    .filter(|p| p.extension().is_some_and(|x| x == "el")).collect();
                names.sort();
                for file in names {
                    let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
                    if let Some(owner) = gnu.iter().map(|g| g.join(format!("{stem}.el"))).find(|f| f.is_file()) {
                        let found = printed(&mut vm, &format!("(locate-file-internal {} load-path '(\".elc\" \".el\"))", lisp_string(&stem)));
                        collisions.push((stem, lisp_string(&owner.to_string_lossy()), found));
                    }
                }
            }
            (printed(&mut vm, "(locate-file-internal \"gnus-dbus\" load-path '(\".elc\" \".el\"))"), path, collisions)
        });
        assert_eq!(dbus, lisp_string(&dir.join("obsolete/gnus-dbus.el").to_string_lossy()), "gnus-dbus is not GNU's file: {path:?}");

        let wrong: Vec<_> = collisions.iter().filter(|(_, gnu, found)| gnu != found).collect();
        assert!(wrong.is_empty(), "bundled files found in place of GNU's: {wrong:?}");
        let first = path.iter().position(|p| p.starts_with(&bundled)).expect("rlisp on load-path");
        assert!(path[..first].iter().all(|p| p.starts_with(&dir))
            && path[first..].iter().all(|p| p.starts_with(&bundled)), "{path:?}");
    }

    /// `--loadup`: GNU's command takes a key only when loadup's GNU
    /// files DEFINED it; a GNU name VM2 already had is not GNU's Lisp, and a
    /// key whose GNU command did not load keeps romacs's self-naming stand-in.
    #[test]
    fn gnu_commands_take_over_only_where_loadup_defined_them() {
        let dir = scratch_lisp("takeover", &[
            ("loadup.el", "(load \"defs\")\n(load \"broken\")\n"),
            ("defs.el", ";;; -*- lexical-binding: t -*-\n(defun next-line () (interactive) (forward-line 1))\n"),
            // Signals at form 1, so its split-window-below never exists.
            ("broken.el", "(car 1)\n(defun split-window-below () (interactive) nil)\n"),
        ]);
        let started = on_evaluator(move || {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
            vm.set_collection_floor(1);
            // A GNU command name the VM defines natively (stands for VM2's own
            // functions under GNU's names): loadup does not touch it.
            vm.eval_source("(defun other-window () (interactive) 'vm2-native)").unwrap();
            let started = initialize_loadup_from(&mut vm, Launch::Loadup, None, &dir).unwrap();
            let bound = |vm: &mut Vm, key: &str| { let v = vm.eval_source(&format!("(key-binding {key})")).unwrap(); v.display().unwrap() };
            let keys: Vec<String> = ["[14]", "[down]", "[16]", "[24 50]", "[24 111]", "[24 5]"].iter().map(|k| bound(&mut vm, k)).collect();
            vm.collect_garbage();
            assert_eq!(bound(&mut vm, "[14]"), "next-line", "binding survives a collection");
            let status = started.status();
            (started.bindings, keys, status)
        });
        let (bindings, keys, status) = started;
        assert_eq!(keys, ["next-line", "next-line", "romacs-gui-previous-line", "romacs-gui-split-below",
                          "romacs-gui-other-window", "romacs-gui-eval-last-sexp"]);
        let gnu: Vec<_> = bindings.iter().filter(|b| b.gnu_defined).map(|b| b.key).collect();
        assert_eq!(gnu, ["[14]", "[down]"]);
        assert!(status.contains("journey keys: 2 GNU, 11 romacs stand-in"), "{status}");
    }

    /// Mode lines after `--loadup`, from a loadup.el whose files are known:
    /// (painted bands, each workspace buffer's own `mode-line-format` or
    /// `GLOBAL` when it has none, the default value, `gnu_mode_line`).
    fn loadup_mode_lines(name: &str, loadup: &'static str) -> (Vec<String>, Vec<String>, String, bool) {
        let dir = scratch_lisp(name, &[("loadup.el", loadup)]);
        on_evaluator(move || {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
            let started = initialize_loadup_from(&mut vm, Launch::Loadup, None, &dir).unwrap();
            let snapshot = vm.frame_snapshot(1200, 800).unwrap();
            let bands = snapshot.windows.iter().take(2)
                .map(|w| w.snapshot.mode_line.as_ref().map_or(String::new(), |b| b.utf8.clone())).collect();
            let locals = ["*scratch*", "*VM evaluations*"].iter().map(|b| printed(&mut vm, &own_mode_line(b))).collect();
            let global = printed(&mut vm, "(default-value 'mode-line-format)");
            (bands, locals, global, started.gnu_mode_line)
        })
    }

    /// Loadup did not reach bindings.el's `mode-line-format`: each workspace
    /// buffer keeps its own format, and its content is the labelled stand-in,
    /// so the screen says bindings.el did not load.
    #[test]
    fn workspace_mode_lines_are_labelled_stand_ins_when_bindings_el_did_not_load() {
        let (bands, locals, global, gnu) = loadup_mode_lines("ml-stand-in", "(setq romacs-test-loadup-ran t)\n");
        assert!(!gnu);
        let stand_in = r#""-" "%z%Z%*%+%@" "  " "%12b" "   " "%p L%l" "   " "[stand-in: no bindings.el]""#;
        assert_eq!(locals, [format!("({stand_in} \"   Rum / VM\")"), format!("({stand_in} \"   evaluated at startup\")")]);
        assert_eq!(global, format!("({stand_in} \"   Rum / VM\")"));
        for (band, name) in bands.iter().zip(["*scratch*", "*VM evaluations*"]) {
            assert!(band.starts_with("---:") && band.contains(name) && band.contains(" L"), "{band}");
            assert!(band.contains("[stand-in: no bindings.el]"), "{band}");
        }
    }

    /// Loadup set `mode-line-format` (as GNU's bindings.el does): GNU's
    /// value wins untouched, globally and in every workspace buffer.
    #[test]
    fn workspace_leaves_gnus_mode_line_format_alone_when_loadup_set_it() {
        let (bands, locals, global, gnu) = loadup_mode_lines("ml-gnu", "(setq mode-line-format '(\"GNU-SET %b\"))\n");
        assert!(gnu);
        assert_eq!(locals, ["GLOBAL", "GLOBAL"]);
        assert_eq!(global, "(\"GNU-SET %b\")");
        for (band, name) in bands.iter().zip(["*scratch*", "*VM evaluations*"]) {
            assert!(band.starts_with(&format!("GNU-SET {name}")), "{band}");
            assert!(!band.contains("stand-in"), "{band}");
        }
    }

    #[test]
    fn workspace_preserves_unchanged_bound_mode_line_defaults() {
        for initial in ["nil", "'(\"PREBOUND %b\")"] {
            let dir = scratch_lisp("ml-prebound", &[("loadup.el", "(set-default 'mode-line-format (default-value 'mode-line-format))\n")]);
            let setup = format!("(setq romacs-test-mode-line {initial}) (set-default 'mode-line-format romacs-test-mode-line)");
            on_evaluator(move || {
                let mut vm = Vm::new();
                vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
                let started = initialize_loadup_from(&mut vm, Launch::Loadup, Some(&setup), &dir).unwrap();
                assert!(started.gnu_mode_line, "an unchanged bound default must not be mistaken for an unset default: {initial}");
                assert_eq!(printed(&mut vm, "(eq (default-value 'mode-line-format) romacs-test-mode-line)"), "t");
                for buffer in ["*scratch*", "*VM evaluations*"] {
                    assert_eq!(printed(&mut vm, &own_mode_line(buffer)), "GLOBAL", "{buffer}: {initial}");
                }
                let snapshot = vm.frame_snapshot(1200, 800).unwrap();
                println!("startup reproduction: initial={initial} gnu_mode_line={} bands={:?}", started.gnu_mode_line,
                    snapshot.windows.iter().take(2).map(|w| w.snapshot.mode_line.as_ref().map(|b| b.utf8.clone())).collect::<Vec<_>>());
                for (window, name) in snapshot.windows.iter().take(2).zip(["*scratch*", "*VM evaluations*"]) {
                    if initial == "nil" {
                        assert!(window.snapshot.mode_line.is_none(), "GNU nil must suppress the mode line: {name}");
                    } else {
                        let band = &window.snapshot.mode_line.as_ref().expect("prebound format paints a mode line").utf8;
                        assert!(band.starts_with(&format!("PREBOUND {name}")), "{band}");
                        assert!(!band.contains("stand-in"), "{band}");
                    }
                }
            });
        }
    }

    #[test]
    fn workspace_leaves_a_gnu_set_nil_mode_line_format_alone() {
        let (bands, locals, global, gnu) = loadup_mode_lines("ml-gnu-nil", "(set-default 'mode-line-format nil)\n");
        assert_eq!((gnu, global.as_str()), (true, "nil"));
        assert_eq!(locals, ["GLOBAL", "GLOBAL"]);
        assert!(bands.iter().all(|b| !b.contains("stand-in")), "{bands:?}");
    }

    /// `--loadup` on this tree: every load site of lisp/loadup.el is
    /// accounted for in order, the drive goes past the old 17-file chain, and
    /// the window keeps a mode line and a cursor.
    #[test]
    fn loadup_launch_runs_all_of_loadup_and_keeps_a_cursor() {
        let (rows, attempted, sites, mode_line, keys, live, ran, bundled) = on_evaluator(|| {
            let mut vm = Vm::new();
            vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
            let started = initialize_loadup(&mut vm, Launch::Loadup, None).unwrap();
            let loadup = started.loadup.as_ref().unwrap();
            let rows: Vec<String> = loadup.rows.iter().map(|r| r.site.name.clone()).collect();
            let attempted = loadup.rows.iter().filter(|r| !matches!(r.outcome, crate::loadup::Outcome::NotReached { .. })).count();
            let text = std::fs::read_to_string(lisp_dir().join("loadup.el")).unwrap();
            let sites: Vec<String> = crate::loadup::load_sites(&text).unwrap().1.into_iter().map(|s| s.name).collect();
            let snapshot = vm.frame_snapshot(1200, 800).unwrap();
            let mode_line = snapshot.windows[0].snapshot.mode_line.is_some();
            let keys: Vec<(bool, String)> = started.bindings.iter().map(|b| {
                let v = vm.eval_source(&format!("(key-binding {})", b.key)).unwrap();
                (b.gnu_defined, v.display().unwrap())
            }).collect();
            // The live path after loadup: the host's selection query, then
            // keys through the same dispatch the GUI uses.
            let ask = |vm: &mut Vm, source: &str| match vm.eval_source(source) {
                Ok(v) => v.display().unwrap(),
                Err(error) => format!("ERR {error}"),
            };
            let hermes = lisp_dir().canonicalize().unwrap().parent().unwrap().join("rlisp").join("hermes");
            let bundled = ask(&mut vm, &format!("(and (member {} load-path) t)", lisp_string(&hermes.to_string_lossy())));
            let mut live = vec![ask(&mut vm, SELECTED_TAIL)];
            let (mut commands, mut presentation) = (crate::command::Commands::default(), crate::presentation::Presentation::new());
            let mut press = |vm: &mut Vm, keys: &[i64]| -> Vec<String> {
                keys.iter().filter_map(|k| presentation.command(vm, &mut commands, romacs_vm::CommandEvent::Character(*k)).unwrap())
                    .map(|o| format!("{}{}", o.command.unwrap_or_default(), o.signal.map(|s| format!(" SIGNAL {}", s.condition)).unwrap_or_default()))
                    .collect()
            };
            let mut ran = vec![];
            for keys in [&[24, 50][..], &[24, 51], &[24, 111], &[24, 49]] {
                ran.extend(press(&mut vm, keys));
                live.push(ask(&mut vm, "(length (romacs-gui-leaves (romacs-gui-root)))"));
            }
            vm.eval_source("(goto-char (point-max))").unwrap();
            ran.extend(press(&mut vm, &[40, 43, 32, 49, 32, 50, 41, 24, 5]));
            live.push(ask(&mut vm, "(let ((old (current-buffer))) (set-buffer (window-buffer (minibuffer-window))) (prog1 (buffer-string) (set-buffer old)))"));
            vm.collect_garbage();
            live.push(ask(&mut vm, SELECTED_TAIL));
            (rows, attempted, sites, mode_line, keys, live, ran, bundled)
        });
        assert_eq!(rows, sites);
        assert!(attempted > GNU_BOOTSTRAP_CHAIN.len(), "loadup stopped inside the old chain: {attempted} sites attempted");
        assert!(mode_line, "the default window has no mode line, so layout would draw no cursor");
        for (gnu, command) in keys {
            assert_eq!(gnu, !command.starts_with("romacs-"), "{command}");
        }
        // Two windows, selected first; C-x 2, C-x 3 split, C-x o moves, C-x 1
        // leaves one; C-x C-e still evaluates; all after GNU's window.el
        // replaced `window-child` with Lisp the VM cannot run.
        // romacs' bundled Lisp is on the load-path after loadup.el set GNU's.
        assert_eq!(bundled, "t", "rlisp/hermes not on load-path after loadup");
        assert_eq!(live, ["2", "3", "4", "4", "1", "\"3\"", "1"], "{ran:?}");
        assert!(ran.iter().all(|r| !r.contains("SIGNAL")), "{ran:?}");
        assert!(ran.contains(&"romacs-gui-eval-last-sexp".to_owned()), "{ran:?}");
    }
}
