//! Drive GNU's `lisp/loadup.el` in its own order, as far as the VM gets.
//!
//! GNU's dump runs `temacs -l loadup`: every top-level form of loadup.el in
//! order, each `(load "FILE")` call loading FILE the way `readevalloop` does.
//! This module runs THE SAME FILE's forms, read from `ROMACS_LISP_DIR`, in the
//! same order, with loadup's own conditions (`featurep`, `fboundp`,
//! `system-type`, `condition-case`) decided by the VM. Its production deviations
//! are stated here and in the loadup report:
//!
//! 1. **A failed load site does not end the run.** GNU's dump aborts at the
//!    first error. Here each `(load ...)` call written in loadup.el is
//!    rewritten to `(romacs-loadup--load SITE ...)`, which calls the real
//!    `load` and, if it signals anything other than a `file-error`, records the
//!    failure and returns nil, so the NEXT load site still runs. `file-error`
//!    is re-signalled because loadup.el's own handler for it
//!    (`(condition-case nil (load "loaddefs") (file-error (load
//!    "ldefs-boot.el")))`) must still see it. Likewise a top-level loadup form
//!    that signals is recorded and the next top-level form runs.
//! 2. **Native declarations replace Custom and faces.** The `custom`,
//!    `cus-face`, and `faces` sites are reported as NATIVE, not complete.
//!    Before `simple`, GNU's `inline` and `derived` libraries supply its
//!    Lisp-owned macro dependencies. Inside each loaded file, loading is GNU's:
//!    the file stops at its first form that signals. At the pin GNU has no
//!    per-form fallback in eager mode either: `internal-macroexpand-for-load`
//!    (macroexp.el:947) signals "Eager macro-expansion failure" and the load
//!    aborts (measured on GNU 32.0.50, see LOADUP.md). So a file is COMPLETE
//!    only if `load` returned normally, which in VM2's loader means every form
//!    was read and evaluated without error.
//!
//! 3. **GNU's `subdirs.el` step of `normal-top-level` is done in Rust.**
//!    loadup.el's last form, `(eval top-level t)`, calls `normal-top-level`
//!    (startup.el), which loads `subdirs.el` from each `load-path` directory
//!    (startup.el:612-638); that is how GNU's path grows past loadup.el's 7
//!    directories to `org/`, `calendar/`, `obsolete/`... A partially loaded
//!    startup can define the function without completing this loop. Unless
//!    the resulting ordered path is already established, [`apply_subdirs`]
//!    supplies it when startup left its input unchanged. The entries
//!    are what GNU's build writes into each `subdirs.el` ([`update_subdirs`],
//!    a port of `build-aux/update-subdirs`, applied where `lisp/Makefile.in`
//!    applies it), spliced exactly as `normal-top-level-add-to-load-path`
//!    (startup.el:472) splices them. DIVERGENCES.md VM2-LP1 says why this is
//!    Rust rather than GNU's own Lisp.
//!
//! Load sites are found in loadup.el's text by [`load_sites`]: an open paren,
//! the symbol `load`, then whitespace, outside strings, comments and character
//! literals. Their number and names are cross-checked against GNU's own reader
//! in the evidence (LOADUP.md).
use romacs_vm::Vm;
use std::path::{Path, PathBuf};

/// One `(load ...)` call written in loadup.el, in source order.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    /// 1-based, source order: the Nth `(load ...)` call in loadup.el's source order.
    pub index: usize,
    /// The literal file argument, e.g. `"emacs-lisp/byte-run"`.
    pub name: String,
    /// 1-based line in loadup.el.
    pub line: usize,
    /// 0-based top-level loadup form that contains it.
    pub form: usize,
    /// Byte offset of the `(` in loadup.el.
    pub byte: usize,
}
romacs_snapshot::snapshot_struct!(Site { index, name, line, form, byte });

/// What became of one load site.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// `load` returned normally: all `forms` top-level forms evaluated.
    Complete { forms: usize },
    /// `load` signalled at form `failed` (1-based); `forms` before it evaluated.
    Partial { forms: usize, failed: usize, line: usize, source: String, error: String, total: Total },
    /// `load` with NOERROR found no file (GNU returns nil the same way).
    Absent,
    /// Native declarations own this subsystem; the GNU file was not loaded.
    Native,
    /// `load` signalled `file-missing` before reading a form: the file is not
    /// in this tree (GNU generates some of these at build time).
    Missing { error: String },
    /// The site was never called. `why` names the loadup form's outcome.
    NotReached { why: String },
}
romacs_snapshot::snapshot_enum!(Outcome {
    Outcome::Complete { forms } => (forms),
    Outcome::Partial { forms, failed, line, source, error, total } => (forms, failed, line, source, error, total),
    Outcome::Absent => (),
    Outcome::Native => (),
    Outcome::Missing { error } => (error),
    Outcome::NotReached { why } => (why),
});

/// How many top-level forms the file has, by VM2's own reader.
#[derive(Clone, Debug, PartialEq)]
pub enum Total {
    Forms(usize),
    /// The reader stopped at this 1-based form; the file has at least that many.
    ReaderStopped { at: usize, error: String },
}
romacs_snapshot::snapshot_enum!(Total {
    Total::Forms(f0) => (f0),
    Total::ReaderStopped { at, error } => (at, error),
});

#[derive(Clone, Debug)]
pub struct Row {
    pub site: Site,
    pub outcome: Outcome,
    /// Files loaded (by `require` or `load`) while this site ran, other than
    /// the site's own file, in order: forms evaluated, and the first failing
    /// form's `line N form K: error` if one failed.
    pub nested: Vec<(String, usize, Option<String>)>,
}
romacs_snapshot::snapshot_struct!(Row { site, outcome, nested });

/// A top-level loadup.el form that signalled (it is not a load site's own
/// failure: those are caught in the wrapper).
#[derive(Clone, Debug)]
pub struct FormFailure {
    pub form: usize,
    pub line: usize,
    pub source: String,
    pub error: String,
}
romacs_snapshot::snapshot_struct!(FormFailure { form, line, source, error });

#[derive(Clone, Debug)]
pub struct Loadup {
    pub loadup: PathBuf,
    /// Top-level forms in loadup.el.
    pub forms: usize,
    pub rows: Vec<Row>,
    pub form_failures: Vec<FormFailure>,
    /// Seconds spent in the drive.
    pub seconds: f64,
    /// The `load-path` directories whose `subdirs.el` [`apply_subdirs`]
    /// applied, or `None` when the ordered subdirs result was already present (so the
    /// step was GNU's, not ours).
    pub subdirs: Option<Vec<String>>,
    /// `load-path` entries whose `subdirs.el` [`apply_subdirs`] did NOT
    /// apply, and why (outside the domain it reproduces; see `portable`).
    pub subdirs_skipped: Vec<(String, String)>,
    /// `(length load-path)` when the drive finished.
    pub load_path_len: usize,
    /// Exact GNU command definitions loaded independently of unrelated bootstrap failures.
    pub command_definitions: Vec<(String, usize)>,
    /// `load-path` when loadup.el reached `(eval top-level t)`, for [`drive`]'s subdirs step.
    before_startup: Option<Vec<String>>,
}
romacs_snapshot::snapshot_struct!(Loadup { loadup, forms, rows, form_failures, seconds, subdirs, subdirs_skipped, load_path_len, command_definitions, before_startup });

impl Loadup {
    pub fn count(&self, pick: fn(&Outcome) -> bool) -> usize {
        self.rows.iter().filter(|r| pick(&r.outcome)).count()
    }
    pub fn complete(&self) -> usize { self.count(|o| matches!(o, Outcome::Complete { .. })) }
    pub fn partial(&self) -> usize { self.count(|o| matches!(o, Outcome::Partial { .. })) }
    pub fn native(&self) -> usize { self.count(|o| matches!(o, Outcome::Native)) }
    pub fn absent(&self) -> usize { self.count(|o| matches!(o, Outcome::Absent)) }
    pub fn missing(&self) -> usize { self.count(|o| matches!(o, Outcome::Missing { .. })) }
    pub fn not_reached(&self) -> usize { self.count(|o| matches!(o, Outcome::NotReached { .. })) }
    /// Where GNU's own dump would have died: the first site or top-level form
    /// that signalled, in execution order.
    pub fn gnu_would_stop(&self) -> Option<String> {
        let site = self.rows.iter().find(|r| matches!(r.outcome, Outcome::Partial { .. }));
        let form = self.form_failures.first();
        let file = self.file_name();
        match (site, form) {
            (Some(r), Some(f)) if f.form < r.site.form => Some(format!("{file} form {} (line {})", f.form + 1, f.line)),
            (Some(r), _) => Some(format!("load site {} \"{}\" ({file} line {})", r.site.index, r.site.name, r.site.line)),
            (None, Some(f)) => Some(format!("{file} form {} (line {})", f.form + 1, f.line)),
            (None, None) => None,
        }
    }
    /// The driven file's name: `loadup.el`, or the shared bootstrap's.
    pub fn file_name(&self) -> String {
        self.loadup.file_name().map_or_else(|| self.loadup.display().to_string(), |n| n.to_string_lossy().into_owned())
    }
    pub fn status(&self) -> String {
        format!(
            "{} ({} top-level forms): {} load sites; {} complete, {} partial, {} missing, {} absent, {} native, {} never reached; {} top-level loadup forms signalled; {}",
            self.loadup.display(), self.forms, self.rows.len(), self.complete(), self.partial(), self.missing(), self.absent(), self.native(), self.not_reached(), self.form_failures.len(),
            match &self.subdirs {
                Some(dirs) => format!("subdirs.el applied by romacs for {} director{}, load-path {} entries{}", dirs.len(), if dirs.len() == 1 { "y" } else { "ies" }, self.load_path_len,
                    if self.subdirs_skipped.is_empty() { String::new() } else {
                        format!(" (NOT APPLIED: {})", self.subdirs_skipped.iter().map(|(d, why)| format!("{d}: {why}")).collect::<Vec<_>>().join("; "))
                    }),
                None => format!("ordered subdirs path is already established; load-path {} entries", self.load_path_len),
            }
        )
    }
    /// One line per site, tab-separated, for the evidence table.
    pub fn table(&self) -> String {
        let mut out = String::from("site\tline\tfile\tstatus\tforms\tdetail\n");
        for row in &self.rows {
            let s = &row.site;
            let (status, forms, detail) = match &row.outcome {
                Outcome::Complete { forms } => ("COMPLETE", forms.to_string(), String::new()),
                Outcome::Partial { forms, failed, line, source, error, total } => {
                    let total = match total {
                        Total::Forms(n) => n.to_string(),
                        Total::ReaderStopped { at, .. } => format!(">={at}"),
                    };
                    ("PARTIAL", format!("{forms}/{total}"),
                     format!("form {failed} line {line}: {} => {}", one_line(source, 90), one_line(error, 240)))
                }
                Outcome::Native => ("NATIVE", "0".into(), "native declarations replace GNU Custom and faces".into()),
                Outcome::Absent => ("ABSENT", "0".into(), "noerror load found no file".into()),
                Outcome::Missing { error } => ("MISSING", "0".into(), format!("file not in this tree: {}", one_line(error, 120))),
                Outcome::NotReached { why } => ("NOT-REACHED", "-".into(), why.clone()),
            };
            let nested = if row.nested.is_empty() { String::new() } else {
                format!(" [nested: {}]", row.nested.iter()
                    .map(|(f, n, failed)| match failed {
                        None => format!("{f} {n}"),
                        Some(e) => format!("{f} {n} FAILED at {}", one_line(e, 160)),
                    })
                    .collect::<Vec<_>>().join(", "))
            };
            out.push_str(&format!("{}\t{}\t{}\t{status}\t{forms}\t{detail}{nested}\n", s.index, s.line, s.name));
        }
        for f in &self.form_failures {
            out.push_str(&format!("form{}\t{}\t({})\tFORM-SIGNALLED\t-\t{} => {}\n",
                f.form + 1, f.line, self.file_name(), one_line(&f.source, 90), one_line(&f.error, 240)));
        }
        for (name, line) in &self.command_definitions {
            out.push_str(&format!("command\t{line}\tsimple.el\tDEFINITION\t1\t{name}; scoped source load, not whole-file completion\n"));
        }
        out
    }
}

fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { format!("{}…", flat.chars().take(max).collect::<String>()) } else { flat }
}

/// Top-level form spans of `source` and the load sites inside them.
/// Returns `(spans, sites)`; a span is `(start, end, line)` of one top-level form.
pub fn load_sites(source: &str) -> Result<(Vec<(usize, usize, usize)>, Vec<Site>), String> {
    scan_forms(source, true)
}

/// Read top-level form boundaries without interpreting nested load calls.
pub fn form_spans(source: &str) -> Result<Vec<(usize, usize, usize)>, String> {
    scan_forms(source, false).map(|(spans, _)| spans)
}

fn scan_forms(source: &str, collect_loads: bool) -> Result<(Vec<(usize, usize, usize)>, Vec<Site>), String> {
    let bytes = source.as_bytes();
    let mut spans = vec![];
    let mut sites = vec![];
    let mut depth = 0usize;
    let mut start = None;
    let mut line = 1;
    let mut i = 0;
    // Whether the previous byte ended a token (so `?` starts a character).
    let mut at_token_start = true;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\n' => { line += 1; i += 1; at_token_start = true; continue; }
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                continue;
            }
            b'"' => {
                if depth == 0 { return Err(format!("loadup.el line {line}: top-level string")); }
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' { i += 1; }
                    if i < bytes.len() && bytes[i] == b'\n' { line += 1; }
                    i += 1;
                }
                i += 1;
                at_token_start = true;
                continue;
            }
            b'?' if at_token_start => {
                i += 1;
                while i < bytes.len() && bytes[i] == b'\\' {
                    i += 1;
                    if bytes.get(i) == Some(&b'^') {
                        i += 1;
                    } else if bytes.get(i).is_some_and(|c| b"CMSHsA".contains(c))
                        && bytes.get(i + 1) == Some(&b'-') {
                        i += 2;
                    } else {
                        break;
                    }
                }
                if i < bytes.len() {
                    if bytes[i] == b'\n' { line += 1; }
                    i += source[i..].chars().next().map_or(1, char::len_utf8);
                }
                at_token_start = false;
                continue;
            }
            b'\\' => { i += 2; at_token_start = false; continue; }
            b'(' => {
                if depth == 0 { start = Some((i, line)); }
                depth += 1;
                let rest = &source[i + 1..];
                if collect_loads && rest.starts_with("load") && rest[4..].starts_with(|c: char| c.is_ascii_whitespace()) {
                    let arg = rest[4..].trim_start();
                    let name = arg.strip_prefix('"').and_then(|a| a.split_once('"')).map(|(n, _)| n.to_owned())
                        .ok_or_else(|| format!("loadup.el line {line}: load site without a literal file name"))?;
                    sites.push(Site { index: sites.len() + 1, name, line, form: spans.len(), byte: i });
                }
                i += 1;
                at_token_start = true;
                continue;
            }
            b')' => {
                depth = depth.checked_sub(1).ok_or_else(|| format!("loadup.el line {line}: unbalanced )"))?;
                i += 1;
                if depth == 0 {
                    let (s, l) = start.take().unwrap();
                    spans.push((s, i, l));
                }
                at_token_start = true;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\x0c' | b'\'' | b'`' | b',' | b'#' => { i += 1; at_token_start = true; continue; }
            _ => {
                if depth == 0 { return Err(format!("loadup.el line {line}: top-level atom")); }
                i += 1;
                at_token_start = false;
            }
        }
    }
    if depth != 0 { return Err("loadup.el: unbalanced (".into()); }
    Ok((spans, sites))
}

/// The wrapper every rewritten load site calls. Plain functions only, defined
/// before GNU's byte-run.el replaces `defun`. Each site appends one entry to
/// `romacs-loadup--log`: (SITE . ok), (SITE . absent) or (SITE error DATA).
/// `romacs-loadup--adapt` enables raw loadup.el's two adaptations (deviation
/// 2); the shared bootstrap states its own, so it runs every site as written.
const WRAPPER: &str = r#"
(setq romacs-loadup--log nil)
(defun romacs-loadup--load (site file &optional noerror nomessage nosuffix must-suffix)
  (condition-case romacs-loadup--error
      (if (and romacs-loadup--adapt (member file '("custom" "cus-face" "faces")))
          (progn (setq romacs-loadup--log (cons (cons site 'native) romacs-loadup--log)) nil)
        (when (and romacs-loadup--adapt (equal file "simple"))
          (load "emacs-lisp/inline")
          (load "emacs-lisp/derived"))
        (if (load file noerror nomessage nosuffix must-suffix)
            (progn (setq romacs-loadup--log (cons (cons site 'ok) romacs-loadup--log)) t)
          (setq romacs-loadup--log (cons (cons site 'absent) romacs-loadup--log))
          nil))
    (error
     (setq romacs-loadup--log
           (cons (cons site (cons 'error romacs-loadup--error)) romacs-loadup--log))
     (if (memq (car romacs-loadup--error) '(file-error file-missing))
         (signal (car romacs-loadup--error) (cdr romacs-loadup--error)))
     nil)))
"#;

/// Rewrite every load site of `form_text` (which starts at `offset` in
/// loadup.el) to call the wrapper with its site index.
fn rewrite(form_text: &str, offset: usize, sites: &[Site]) -> String {
    let mut out = String::with_capacity(form_text.len() + 64);
    let mut at = 0;
    for site in sites {
        let local = site.byte - offset;
        out.push_str(&form_text[at..local]);
        out.push_str(&format!("(romacs-loadup--load {}", site.index));
        at = local + "(load".len();
    }
    out.push_str(&form_text[at..]);
    out
}

/// Run loadup.el from `dir` in this VM. `load-path` is `(DIR)`, as GNU's
/// temacs has it before loadup.el adds its own subdirectories.
pub fn drive(vm: &mut Vm, dir: &Path) -> Result<Loadup, String> {
    let begun = std::time::Instant::now();
    let dir = dir.canonicalize().map_err(|e| format!("GNU lisp directory {}: {e}", dir.display()))?;
    vm.set_load_path(std::slice::from_ref(&dir))?;
    let mut loadup = run(vm, &dir.join("loadup.el"), std::slice::from_ref(&dir), true)?;
    let current_path = load_path(vm)?;
    let initial_path = loadup.before_startup.take().unwrap_or_else(|| current_path.clone());
    let (path, applied, mut subdirs_skipped) = apply_subdirs(initial_path.clone(), &dir);
    // A defined normal-top-level is not evidence that its subdirs loop ran.
    // Compare the whole ordered result: replaying a completed loop duplicates
    // entries, while accepting the unexpanded prefix hides GNU libraries.
    loadup.subdirs = if current_path == path {
        None
    } else if current_path == initial_path {
        vm.set_load_path(&path.iter().map(PathBuf::from).collect::<Vec<_>>())?;
        Some(applied)
    } else {
        subdirs_skipped.push(("load-path".into(),
            "startup left neither its initial path nor the completed subdirs path".into()));
        Some(vec![])
    };
    loadup.subdirs_skipped = subdirs_skipped;
    loadup.load_path_len = load_path(vm)?.len();
    if loadup.rows.iter().any(|row| row.site.name == "simple" && matches!(row.outcome, Outcome::Partial { .. })) {
        let source = std::fs::read_to_string(dir.join("simple.el")).map_err(|e| e.to_string())?;
        loadup.command_definitions = load_line_command_definitions(vm, &source)?;
    }
    loadup.seconds = begun.elapsed().as_secs_f64();
    Ok(loadup)
}

/// GNU's installed `load-path` for the Lisp directory `dir`: `(DIR)` grown by
/// `normal-top-level`'s subdirs loop ([`apply_subdirs`]), as a dumped GNU
/// Emacs starts with it.
pub fn installed_load_path(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = dir.canonicalize().map_err(|e| format!("GNU lisp directory {}: {e}", dir.display()))?;
    let (path, _, skipped) = apply_subdirs(vec![dir.to_string_lossy().into_owned()], &dir);
    if let Some((entry, why)) = skipped.first() { return Err(format!("subdirs.el of {entry}: {why}")); }
    Ok(path.into_iter().map(PathBuf::from).collect())
}

/// Run `bootstrap` (rlisp/romacs-loadup-bootstrap.el, the chain the batch
/// package runner preloads) on `load_path`, as that runner does: the path is
/// already GNU's installed one, so the drive adds no subdirs step and no
/// adaptation of its own; the file's adaptations are its forms. Deviation 1
/// still holds: a failing site is recorded and the next form runs.
pub fn drive_bootstrap(vm: &mut Vm, bootstrap: &Path, load_path: &[PathBuf]) -> Result<Loadup, String> {
    let begun = std::time::Instant::now();
    vm.set_load_path(load_path)?;
    let mut loadup = run(vm, bootstrap, load_path, false)?;
    loadup.load_path_len = load_path.len();
    loadup.seconds = begun.elapsed().as_secs_f64();
    Ok(loadup)
}

/// Every top-level form of `file` in order, its load sites through the
/// wrapper; each site's file is found on `search` as `load` finds it.
fn run(vm: &mut Vm, file: &Path, search: &[PathBuf], adapt: bool) -> Result<Loadup, String> {
    // Through the host boundary, so a world build records the file it ran.
    let loadup = romacs_vm::host::canonicalize(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let source = romacs_vm::host::read_to_string(&loadup).map_err(|e| format!("{}: {e}", loadup.display()))?;
    let (spans, sites) = load_sites(&source)?;
    let file_name = loadup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // VM2's own runtime (its Lisp-written primitives), as every GUI path installs.
    vm.install_lisp_prelude()?;
    vm.eval_source(WRAPPER)?;
    vm.eval_source(&format!("(setq romacs-loadup--adapt {})", if adapt { "t" } else { "nil" }))?;
    let mut form_failures = vec![];
    let mut form_errors: Vec<Option<String>> = vec![None; spans.len()];
    // Steps of the VM's loader, per site invocation: (site, from, to).
    let mut windows: Vec<(usize, usize, usize)> = vec![];
    let mut before_startup = None;
    for (k, &(start, end, line)) in spans.iter().enumerate() {
        if source[start..end].split_whitespace().collect::<String>() == "(evaltop-levelt)" {
            before_startup = Some(load_path(vm)?);
        }
        let inside: Vec<Site> = sites.iter().filter(|s| s.form == k).cloned().collect();
        let text = rewrite(&source[start..end], start, &inside);
        let from = steps_len(vm);
        let logged = log_len(vm)?;
        // The form is the site of what it reads, as when the loader runs it.
        let site = romacs_vm::host::form_site(&loadup, line, source[start..end].trim_start().lines().next().unwrap_or(""));
        let outcome = vm.eval_source(&text).map(|_| ());
        drop(site);
        let to = steps_len(vm);
        let invoked = invoked_since(vm, logged)?;
        // Attribute this form's loader steps to the sites it invoked, in order.
        let mut cursor = from;
        for site in &invoked {
            let path = resolve(search, &sites[site - 1].name);
            match first_step(vm, cursor, to, path.as_deref()) {
                Some(begin) => { windows.push((*site, begin, to)); cursor = begin + 1; }
                // No step of its own (missing file, or stopped before form 1).
                None => windows.push((*site, to, to)),
            }
        }
        if let Err(error) = outcome {
            form_errors[k] = Some(error.clone());
            form_failures.push(FormFailure { form: k, line, source: source[start..end].to_owned(), error });
        }
    }
    // Close each site's window at the next site's own start.
    for i in 0..windows.len() {
        if let Some(next) = windows[i + 1..].iter().map(|w| w.1).find(|&n| n > windows[i].1) {
            windows[i].2 = windows[i].2.min(next);
        }
    }
    let report = vm.load_report();
    let steps = report["steps"].as_array().cloned().unwrap_or_default();
    let mut rows = vec![];
    for site in &sites {
        let entry = log_entry(vm, site.index)?;
        let path = resolve(search, &site.name);
        let window = windows.iter().find(|w| w.0 == site.index).map(|w| (w.1, w.2));
        let (own, nested) = match (window, path.as_ref()) {
            (Some((from, to)), Some(path)) => split_steps(&steps[from.min(steps.len())..to.min(steps.len())], path),
            _ => (vec![], vec![]),
        };
        let outcome = match entry.as_deref() {
            None => NotReachedWhy::explain(&form_errors, site, &file_name),
            Some("ok") => Outcome::Complete { forms: own.iter().filter(|s| s["status"] == "ok").count() },
            Some("absent") => Outcome::Absent,
            Some("native") => Outcome::Native,
            Some(error) if own.is_empty() && path.is_none() && error.starts_with("(file-missing") => {
                Outcome::Missing { error: error.to_owned() }
            }
            Some(error) => {
                let ok = own.iter().filter(|s| s["status"] == "ok").count();
                let failed = own.iter().find(|s| s["status"] != "ok");
                let (line, source, step_error) = failed.map(|s| (
                    s["line"].as_u64().unwrap_or(0) as usize,
                    s["source"].as_str().unwrap_or("").to_owned(),
                    s["error"].as_str().unwrap_or("").to_owned(),
                )).unwrap_or((0, String::new(), String::new()));
                let total = path.as_ref().map(|p| total_forms(vm, p)).unwrap_or(Total::Forms(0));
                // No failing step: the loader's READER stopped before the form
                // was entered (VM2's loader records a step only once read).
                let step_error = match (&failed, &total) {
                    (None, Total::ReaderStopped { at, error }) if *at == ok + 1 => format!("reader: {error}"),
                    _ => step_error,
                };
                let error = if step_error.is_empty() { error.to_owned() } else { format!("{step_error} | condition {error}") };
                Outcome::Partial { forms: ok, failed: ok + 1, line, source, error, total }
            }
        };
        rows.push(Row { site: site.clone(), outcome, nested });
    }
    Ok(Loadup { loadup, forms: spans.len(), rows, form_failures, seconds: 0.0, subdirs: None, subdirs_skipped: vec![],
                load_path_len: 0, command_definitions: vec![], before_startup })
}

fn load_line_command_definitions(vm: &mut Vm, source: &str) -> Result<Vec<(String, usize)>, String> {
    let mut loaded = vec![];
    let (spans, _) = load_sites(source)?;
    let declarations = [
        ("defcustom", "track-eol"), ("defcustom", "goal-column"),
        ("defvar", "temporary-goal-column"), ("defcustom", "line-move-ignore-invisible"),
        ("defcustom", "line-move-visual"),
        ("defun", "line-move-partial"), ("defun", "line-move"),
        ("defun", "line-move-visual"), ("defun", "line-move-1"),
        ("defun", "line-move-finish"), ("defun", "line-move-to-column"),
        ("defun", "move-beginning-of-line"), ("defun", "move-end-of-line"),
    ];
    for (kind, name) in declarations {
        let predicate = if kind == "defun" { "fboundp" } else { "boundp" };
        if vm.eval_source(&format!("({predicate} '{name})"))?.display()? != "nil" { continue; }
        let prefix = format!("({kind} {name}");
        let definitions: Vec<_> = spans.iter().filter(|&&(start, end, _)| {
            source[start..end].strip_prefix(&prefix)
                .is_some_and(|tail| tail.starts_with(char::is_whitespace))
        }).collect();
        if definitions.len() > 1 { return Err(format!("ambiguous GNU definition: {name}")); }
        if let Some(&&(start, end, line)) = definitions.first() {
            vm.eval_source(&source[start..end])?;
            loaded.push((name.to_owned(), line));
        }
    }
    Ok(loaded)
}

/// `load-path` as strings.
pub fn load_path(vm: &mut Vm) -> Result<Vec<String>, String> {
    let n: usize = vm.eval_source("(length load-path)")?.display()?.parse().map_err(|e| format!("load-path length: {e}"))?;
    (0..n).map(|i| {
        let printed = vm.eval_source(&format!("(nth {i} load-path)"))?.display()?;
        serde_json::from_str::<String>(&printed).map_err(|e| format!("load-path element {i} {printed}: {e}"))
    }).collect()
}

/// Reproduce GNU's `build-aux/update-subdirs DIR` run in the C locale
/// (see [`SubdirsEl`]; `None` where it writes no file). A port
/// of the script's loop (update-subdirs:24-37 at the pin): over `*` (glob
/// order; no dotfiles), skip `*.elc *.el term RCS CVS Old =* *~ *.orig *.rej`
/// and non-directories, PREPEND each name, except `obsolete`, which is
/// APPENDED, so the list is reverse glob order with `obsolete` last. We
/// always use C-locale byte order, regardless of the GNU builder's locale.
/// Non-C GNU builds can order mixed-case/punctuation names differently,
/// changing load-path precedence; the pin's names agree under en_US.UTF-8
/// (measured, LOADUP.md).
pub fn update_subdirs(dir: &Path) -> Result<SubdirsEl, String> {
    use std::os::unix::ffi::OsStrExt;
    let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?
        .map(|e| e.map(|e| e.file_name()).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    names.retain(|n| !n.as_bytes().starts_with(b"."));
    names.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let (mut list, mut order, mut odd) = (String::new(), std::collections::VecDeque::new(), vec![]);
    for file in names {
        let b = file.as_bytes();
        let skipped = [&b".elc"[..], b".el", b"~", b".orig", b".rej"].iter().any(|s| b.ends_with(s))
            || [&b"term"[..], b"RCS", b"CVS", b"Old"].contains(&b) || b.starts_with(b"=");
        if skipped || !dir.join(&file).is_dir() { continue; }
        if !portable(b) { odd.push(file.to_string_lossy().into_owned()); continue; }
        let file = file.to_string_lossy().into_owned();
        // ONE decision per name for both the text and the names.
        let append = file == "obsolete";
        list = if append { format!("{list} \"{file}\"") } else { format!("\"{file}\" {list}") };
        if append { order.push_back(file) } else { order.push_front(file) }
    }
    Ok(if !odd.is_empty() {
        SubdirsEl::Unsupported(format!("subdirectory names outside the reproduced domain: {}", odd.join(", ")))
    } else if list.is_empty() { SubdirsEl::None } else { SubdirsEl::File { list, names: order.into() } })
}

/// One directory's `subdirs.el` as GNU's build leaves it.
#[derive(Clone, Debug, PartialEq)]
pub enum SubdirsEl {
    /// GNU's build writes none here.
    None,
    /// The script's `$subdirs` string (built by its own string operations)
    /// and the names in it, in order.
    File { list: String, names: Vec<String> },
    /// A name or path outside [`portable`]'s domain: GNU's result is not
    /// reproduced, so NOTHING of this directory's file is applied, and the
    /// drive reports why.
    Unsupported(String),
}

/// The names whose whole path through GNU's build and startup the port
/// reproduces: ASCII letters, digits and `-_.+,@=~`, not starting with `~`.
/// Outside it GNU does things the port does not model: the script writes
/// `"NAME"` unescaped, so the Lisp reader decodes a backslash (`a\z` reads
/// `az`) and a `"` ends the string and breaks the whole form;
/// `expand-file-name` makes `~root` `/root`; lisp/Makefile.in passes the
/// directory set as unquoted shell words, so whitespace or shell syntax in a
/// path splits or expands it; and names need not be UTF-8. The pin's names
/// are all inside it.
fn portable(name: &[u8]) -> bool {
    !name.is_empty() && name[0] != b'~' && name.iter().all(|&b| b.is_ascii_alphanumeric() || b"-_.+,@=~".contains(&b))
}

/// The `subdirs.el` GNU's build leaves in a `load-path` directory:
/// `make -C lisp update-subdirs` (run before bootstrap-emacs is dumped,
/// src/Makefile.in:961) runs the script on every directory of `lisp/` except
/// `cedet%` and `leim%` (`SUBDIRS_SUBDIRS`, lisp/Makefile.in:131,223-226).
/// Directories outside `lisp/` have none. The Makefile's set is `find -type
/// d` (lisp/Makefile.in:121), which does not follow symlinks, so the file
/// seen through a symlinked directory is its TARGET's: decide on the
/// canonical path (`lisp` is canonical).
fn gnu_subdirs_el(lisp: &Path, dir: &Path) -> SubdirsEl {
    use std::os::unix::ffi::OsStrExt;
    let Ok(real) = dir.canonicalize() else { return SubdirsEl::None };
    let Ok(rel) = real.strip_prefix(lisp) else { return SubdirsEl::None };
    if !rel.components().all(|c| portable(c.as_os_str().as_bytes())) {
        return SubdirsEl::Unsupported(format!("directory {} is outside the reproduced domain", rel.display()));
    }
    let rel = rel.to_string_lossy();
    if rel.starts_with("cedet") || rel.starts_with("leim") { return SubdirsEl::None; }
    // An unreadable directory: the script's `cd` fails and writes nothing.
    update_subdirs(&real).unwrap_or(SubdirsEl::None)
}

/// GNU's `normal-top-level-add-to-load-path` (startup.el:472-486): find
/// `(directory-file-name default-directory)` in `load-path`, stopping at the
/// LAST element if it is absent, and splice the expanded `dirs` right after
/// it. Returns the index spliced after. For [`portable`] names relative to
/// an absolute directory, `expand-file-name` is a plain join.
fn add_to_load_path(load_path: &mut Vec<String>, default_directory: &str, dirs: &[String]) -> Option<usize> {
    if load_path.is_empty() { return None; }
    let trimmed = default_directory.trim_end_matches('/');
    let thisdir = if trimmed.is_empty() && default_directory.starts_with('/') { "/" } else { trimmed };
    let mut at = 0;
    while at + 1 < load_path.len() && load_path[at] != thisdir { at += 1; }
    let expanded = dirs.iter().map(|d| Path::new(default_directory).join(d).to_string_lossy().into_owned());
    load_path.splice(at + 1..at + 1, expanded);
    Some(at)
}

/// `normal-top-level`'s subdirs loop (startup.el:612-638): for each
/// `load-path` entry in turn, INCLUDING entries an earlier `subdirs.el`
/// added, apply that directory's `subdirs.el` with `default-directory` bound
/// to the entry. Each directory's file is applied as GNU applies it for a
/// C-locale build (see [`update_subdirs`]), or not at all and reported.
/// Returns the new path, the entries whose file
/// applied, and `(entry, why)` for each one not applied.
pub fn apply_subdirs(mut load_path: Vec<String>, lisp: &Path) -> (Vec<String>, Vec<String>, Vec<(String, String)>) {
    let mut applied = vec![];
    let mut skipped = vec![];
    let mut i = 0;
    while i < load_path.len() {
        let dir = load_path[i].clone();
        match gnu_subdirs_el(lisp, Path::new(&dir)) {
            SubdirsEl::None => {}
            SubdirsEl::Unsupported(why) => skipped.push((dir, why)),
            SubdirsEl::File { names, .. } => {
                let before = load_path.len();
                // GNU's TAIL is a cons: a splice before it moves it along.
                if add_to_load_path(&mut load_path, &dir, &names).is_some_and(|at| at < i) { i += load_path.len() - before; }
                applied.push(dir);
            }
        }
        i += 1;
    }
    (load_path, applied, skipped)
}

struct NotReachedWhy;
impl NotReachedWhy {
    fn explain(form_errors: &[Option<String>], site: &Site, file: &str) -> Outcome {
        let why = match &form_errors[site.form] {
            Some(error) => format!("{file} form {} signalled before this call: {}", site.form + 1, one_line(error, 200)),
            None => format!("{file} form {} completed without calling it (its condition was false in the VM)", site.form + 1),
        };
        Outcome::NotReached { why }
    }
}

fn steps_len(vm: &Vm) -> usize {
    vm.load_report()["steps"].as_array().map_or(0, Vec::len)
}

fn log_len(vm: &mut Vm) -> Result<usize, String> {
    let v = vm.eval_source("(length romacs-loadup--log)")?;
    v.display()?.parse::<usize>().map_err(|e| e.to_string())
}

/// Site indices appended to the log since it had `before` entries, oldest first.
fn invoked_since(vm: &mut Vm, before: usize) -> Result<Vec<usize>, String> {
    let now = log_len(vm)?;
    let v = vm.eval_source(&format!(
        "(let ((l romacs-loadup--log) (n {}) (out nil)) (while (> n 0) (setq out (cons (car (car l)) out) l (cdr l) n (- n 1))) out)",
        now - before
    ))?;
    let text = v.display()?;
    Ok(text.trim_matches(|c| c == '(' || c == ')').split_whitespace().filter_map(|n| n.parse().ok()).collect())
}

/// "ok", "absent", the printed error data, or None when never called.
fn log_entry(vm: &mut Vm, site: usize) -> Result<Option<String>, String> {
    let v = vm.eval_source(&format!(
        "(let ((e (assq {site} romacs-loadup--log))) (cond ((null e) nil) ((eq (cdr e) 'ok) \"ok\") ((eq (cdr e) 'absent) \"absent\") ((eq (cdr e) 'native) \"native\") (t (format \"%S\" (cdr (cdr e))))))"
    ));
    let v = match v {
        Ok(v) => v,
        // `format` may itself be broken by a partly loaded GNU file; fall back to prin1 of the data.
        Err(_) => vm.eval_source(&format!(
            "(let ((e (assq {site} romacs-loadup--log))) (cond ((null e) nil) ((eq (cdr e) 'ok) \"ok\") ((eq (cdr e) 'absent) \"absent\") ((eq (cdr e) 'native) \"native\") (t (prin1-to-string (cdr (cdr e))))))"
        ))?,
    };
    if v == romacs_vm::Fixed::NIL { return Ok(None); }
    let printed = v.display()?;
    Ok(Some(serde_json::from_str::<String>(&printed).unwrap_or(printed)))
}

/// Where `(load NAME)` finds NAME on `search`: per directory, NAME.el, then NAME.
fn resolve(search: &[PathBuf], name: &str) -> Option<PathBuf> {
    search.iter().flat_map(|dir| [dir.join(format!("{name}.el")), dir.join(name)])
        .find(|p| p.is_file()).and_then(|p| p.canonicalize().ok())
}

fn first_step(vm: &Vm, from: usize, to: usize, path: Option<&Path>) -> Option<usize> {
    let path = path?.to_string_lossy().into_owned();
    let report = vm.load_report();
    let steps = report["steps"].as_array()?;
    (from..to.min(steps.len())).find(|&i| steps[i]["file"] == path.as_str() && steps[i]["ordinal"] == 1)
}

/// Split a site's step window into its own file's steps and nested files.
fn split_steps(window: &[serde_json::Value], path: &Path) -> (Vec<serde_json::Value>, Vec<(String, usize, Option<String>)>) {
    let own_name = path.to_string_lossy();
    let mut own = vec![];
    let mut nested: Vec<(String, usize, Option<String>)> = vec![];
    let mut started = false;
    for step in window {
        let file = step["file"].as_str().unwrap_or("");
        if file == own_name {
            if step["ordinal"] == 1 && started { break; }
            started = true;
            own.push(step.clone());
        } else {
            let short = file.rsplit("/lisp/").next().unwrap_or(file).to_owned();
            let ok = step["status"] == "ok";
            let failure = (!ok).then(|| format!("line {} form {}: {}", step["line"], step["ordinal"],
                                                 step["error"].as_str().unwrap_or("(entered, never finished)")));
            match nested.last_mut() {
                Some(last) if last.0 == short && step["ordinal"] != 1 => {
                    if ok { last.1 += 1 } else if last.2.is_none() { last.2 = failure }
                }
                _ => nested.push((short, usize::from(ok), failure)),
            }
        }
    }
    (own, nested)
}

/// Top-level forms in FILE by the VM's reader (reading only; nothing is evaluated).
fn total_forms(vm: &mut Vm, path: &Path) -> Total {
    let Ok(bytes) = std::fs::read(path) else { return Total::Forms(0) };
    let Ok(text) = String::from_utf8(bytes) else { return Total::ReaderStopped { at: 1, error: "not UTF-8".into() } };
    let mut at = 0;
    let mut n = 0;
    loop {
        match vm.read_source(&text[at..]) {
            Ok(Some((_, used))) => { n += 1; at += used; }
            Ok(None) => return Total::Forms(n),
            Err(error) => return Total::ReaderStopped { at: n + 1, error },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_spans_do_not_interpret_dynamic_loads() {
        let source = "(defun example (file) (load file))\n(defvar example-format (lambda (x) x))";
        let spans = form_spans(source).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(&source[spans[0].0..spans[0].1], "(defun example (file) (load file))");
        assert!(load_sites(source).unwrap_err().contains("without a literal file name"));
    }

    #[test]
    fn native_declarations_exclude_custom_and_face_load_sites() {
        on_evaluator(|| {
            let dir = std::env::temp_dir().join(format!("romacs-native-loadup-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("loadup.el"), "(load \"custom\")\n(load \"cus-face\")\n(load \"faces\")\n").unwrap();
            for name in ["custom", "cus-face", "faces"] {
                std::fs::write(dir.join(format!("{name}.el")), "(error \"excluded source executed\")").unwrap();
            }
            let mut vm = Vm::new();
            let report = drive(&mut vm, &dir).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            assert_eq!(report.native(), 3);
            assert_eq!(report.complete(), 0);
            assert_eq!(report.partial(), 0);
            assert!(report.form_failures.is_empty());
            assert!(vm.load_report()["steps"].as_array().unwrap().is_empty());
            assert_eq!(vm.eval_source("(progn (defcustom native-option 7 \"Option\" :type 'integer) native-option)").unwrap().display().unwrap(), "7");
        });
    }

    #[test]
    fn scoped_line_definitions_preserve_source_and_existing_commands() {
        let mut vm = Vm::new();
        let source = "(error \"unrelated declaration must not run\")\n(defun move-beginning-of-line-extra () 99)\n(defun move-beginning-of-line (arg) (interactive \"p\") (+ arg 7))\n(defun move-end-of-line (arg) (interactive \"p\") (+ arg 11))\n";
        assert_eq!(load_line_command_definitions(&mut vm, source).unwrap(),
                   [("move-beginning-of-line".into(), 3), ("move-end-of-line".into(), 4)]);
        assert_eq!(vm.eval_source("(list (move-beginning-of-line 1) (move-end-of-line 2))").unwrap().display().unwrap(), "(8 13)");
        assert_eq!(vm.eval_source("(fboundp 'move-beginning-of-line-extra)").unwrap().display().unwrap(), "nil");
        assert!(load_line_command_definitions(&mut vm, "(defun move-beginning-of-line (arg) 99)").unwrap().is_empty());
        assert_eq!(vm.eval_source("(move-beginning-of-line 1)").unwrap().display().unwrap(), "8");
        let mut empty = Vm::new();
        assert!(load_line_command_definitions(&mut empty, "(defun move-beginning-of-line () 1) (defun move-beginning-of-line () 2)").unwrap_err().contains("ambiguous"));
    }

    #[test]
    fn load_sites_character_escapes_preserve_form_boundaries() {
        for literal in [r"?\n", r"?\s", r"?\(", r"?\)", r#"?\""#, r"?\\",
                        r"?\C-m", r"?\C-\(", r"?\M-\^@", r"?\^H", r"?\λ", "?λ"] {
            let form = format!("(setq c {literal})");
            let source = format!("{form}\n(load \"after\")");
            let (spans, sites) = load_sites(&source).unwrap_or_else(|error| panic!("{literal}: {error}"));
            assert_eq!(spans, [(0, form.len(), 1), (form.len() + 1, source.len(), 2)], "{literal}");
            assert_eq!(sites.len(), 1, "{literal}");
            assert_eq!((sites[0].name.as_str(), sites[0].form, sites[0].line), ("after", 1, 2));
        }
    }

    #[test]
    fn load_sites_scans_unmodified_simple_el() {
        let source = std::fs::read_to_string(crate::startup::lisp_dir().join("simple.el")).unwrap();
        let (spans, _) = load_sites(&source).unwrap();
        for name in ["move-beginning-of-line", "move-end-of-line"] {
            let prefix = format!("(defun {name} ");
            assert_eq!(spans.iter().filter(|&&(start, end, _)| source[start..end].starts_with(&prefix)).count(), 1, "{name}");
        }
        assert_eq!(&source[spans.last().unwrap().0..spans.last().unwrap().1], "(provide 'simple)");
    }

    #[test]
    fn load_sites_skip_strings_comments_and_characters() {
        let src = r#"
;; (load "commented")
(setq a "(load \"in-string\")")
(setq c ?\( d ?\" e ?\C-\( f ?;)
(load "one")
(if (featurep 'x)
    (progn (load "two" t) (message "x")))
(condition-case nil (load "three") (file-error (load "four.el")))
(setq unload 1 reload (overload "no"))
"#;
        let (spans, sites) = load_sites(src).unwrap();
        assert_eq!(spans.len(), 6);
        let names: Vec<_> = sites.iter().map(|s| (s.name.as_str(), s.form, s.line)).collect();
        assert_eq!(names, [("one", 2, 5), ("two", 3, 7), ("three", 4, 8), ("four.el", 4, 8)]);
        let form = &src[spans[4].0..spans[4].1];
        let inside: Vec<_> = sites.iter().filter(|s| s.form == 4).cloned().collect();
        assert_eq!(rewrite(form, spans[4].0, &inside),
                   r#"(condition-case nil (romacs-loadup--load 3 "three") (file-error (romacs-loadup--load 4 "four.el")))"#);
    }

    fn on_evaluator<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap()
    }

    #[test]
    fn subdirs_completion_requires_established_path_not_function_binding() {
        on_evaluator(|| {
            for established in [false, true] {
                let dir = std::env::temp_dir().join(format!(
                    "romacs-subdirs-completion-{}-{established}", std::process::id()
                ));
                std::fs::create_dir_all(&dir).unwrap();
                for name in ["a", "b", "obsolete", "term"] {
                    std::fs::create_dir_all(dir.join(name)).unwrap();
                }
                let dir = dir.canonicalize().unwrap();
                let expected: Vec<String> = ["", "b", "a", "obsolete"].iter()
                    .map(|name| if name.is_empty() { dir.clone() } else { dir.join(name) })
                    .map(|path| path.to_string_lossy().into_owned()).collect();
                let body = if established {
                    format!("(setq load-path '({}))", expected.iter()
                        .map(|p| format!("{p:?}")).collect::<Vec<_>>().join(" "))
                } else { "(car 1)".to_owned() };
                std::fs::write(dir.join("loadup.el"), format!(
                    "(defun normal-top-level () {body})\n(setq top-level '(normal-top-level))\n(eval top-level t)\n"
                )).unwrap();
                let mut vm = Vm::new();
                let report = drive(&mut vm, &dir).unwrap();
                let path = load_path_of(&mut vm);
                std::fs::remove_dir_all(&dir).unwrap();
                assert_eq!(path, expected,
                    "SUBDIRS_COMPLETION_PATH: bound-but-failed startup needs fallback; completed startup must not duplicate entries (established={established})");
                assert_eq!(report.subdirs.is_none(), established,
                    "SUBDIRS_COMPLETION_AUTHORITY: only established path suppresses fallback");
            }
        });
    }

    /// The deviation and the regime, on files whose behaviour is known:
    /// a failing FORM ends its FILE (GNU's readevalloop), a failing load SITE
    /// does not end the drive (romacs's deviation), a false condition never
    /// calls its site, and loadup's own `file-error` handler still runs.
    #[test]
    fn failed_form_ends_its_file_but_not_the_drive() {
        let dir = std::env::temp_dir().join(format!("romacs-loadup-drive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in [
            ("loadup.el", r#";;; -*- lexical-binding: t -*-
(load "a")
(load "b")
(load "c")
(if (featurep 'no-such-feature) (load "d"))
(condition-case nil (load "missing") (file-error (load "e.el")))
(setq loadup-tail (list 'after (car 1)))
(load "f")
"#),
            ("a.el", "(setq a-1 t)\n(setq a-2 t)\n"),
            ("b.el", "(setq b-1 t)\n(car 1)\n(setq b-3 t)\n"),
            ("c.el", "(setq c-1 t)\n"),
            ("d.el", "(setq d-1 t)\n"),
            ("e.el", "(setq e-1 t)\n"),
            ("f.el", "(setq f-1 t)\n"),
        ] { std::fs::write(dir.join(file), text).unwrap(); }
        let (report, vars) = on_evaluator(move || {
            let mut vm = Vm::new();
            vm.set_collection_floor(1);
            let report = drive(&mut vm, &dir).unwrap();
            let v = vm.eval_source("(list (boundp 'a-2) (boundp 'b-1) (boundp 'b-3) (boundp 'c-1) (boundp 'd-1) (boundp 'e-1) (boundp 'f-1))").unwrap();
            (report, v.display().unwrap())
        });
        // b's third form never ran: the file stopped at form 2, as GNU's would.
        assert_eq!(vars, "(t t nil t nil t t)", "{}", report.table());
        let outcomes: Vec<(&str, &Outcome)> = report.rows.iter().map(|r| (r.site.name.as_str(), &r.outcome)).collect();
        assert!(matches!(outcomes[0], ("a", Outcome::Complete { forms: 2 })), "{outcomes:?}");
        assert!(matches!(outcomes[1], ("b", Outcome::Partial { forms: 1, failed: 2, line: 2, total: Total::Forms(3), .. })), "{outcomes:?}");
        assert!(matches!(outcomes[2], ("c", Outcome::Complete { forms: 1 })), "{outcomes:?}");
        assert!(matches!(outcomes[3], ("d", Outcome::NotReached { .. })), "{outcomes:?}");
        assert!(matches!(outcomes[4], ("missing", Outcome::Missing { .. })), "{outcomes:?}");
        assert!(matches!(outcomes[5], ("e.el", Outcome::Complete { forms: 1 })), "{outcomes:?}");
        assert!(matches!(outcomes[6], ("f", Outcome::Complete { forms: 1 })), "{outcomes:?}");
        assert_eq!(report.form_failures.iter().map(|f| f.form).collect::<Vec<_>>(), [5]);
        assert_eq!(report.gnu_would_stop().as_deref(), Some("load site 2 \"b\" (loadup.el line 3)"));
        assert_eq!((report.complete(), report.partial(), report.missing(), report.not_reached()), (4, 1, 1, 1));
    }

    /// `load-path` as strings, read through `nth` (no GNU list function is
    /// assumed to work after a partial loadup).
    fn load_path_of(vm: &mut Vm) -> Vec<String> {
        let n: usize = vm.eval_source("(length load-path)").unwrap().display().unwrap().parse().unwrap();
        (0..n).map(|i| {
            let v = vm.eval_source(&format!("(nth {i} load-path)")).unwrap();
            serde_json::from_str::<String>(&v.display().unwrap()).unwrap()
        }).collect()
    }

    /// GNU's `lisp/` immediate subdirectories at the pin, in the order GNU's
    /// `build-aux/update-subdirs` writes them into `lisp/subdirs.el`: reverse
    /// glob order, `obsolete` LAST, `term` excluded. Byte-compared against the
    /// script's own output on this tree in evidence/loadup/LOADUP.md.
    const GNU_SUBDIRS: [&str; 24] = [
        "vc", "use-package", "url", "textmodes", "progmodes", "play", "org", "nxml", "net", "mh-e",
        "mail", "leim", "language", "international", "image", "gnus", "eshell", "erc", "emulation",
        "emacs-lisp", "cedet", "calendar", "calc", "obsolete",
    ];
    /// loadup.el's own splice (`lisp/loadup.el:57-99`, taken when `dump-mode`
    /// is nil as in `temacs -l loadup`), which `subdirs.el` does not remove.
    const LOADUP_SUBDIRS: [&str; 6] = ["emacs-lisp", "progmodes", "language", "international", "textmodes", "vc"];

    /// GNU extends `load-path` beyond loadup's 7 directories in
    /// `normal-top-level` (startup.el:612-638), which loads each load-path
    /// directory's `subdirs.el`; `normal-top-level-add-to-load-path`
    /// (startup.el:472) splices its entries in right after that directory.
    /// loadup.el's last form never reaches `normal-top-level` in the VM, so
    /// without the drive's subdirs step Org was not even findable.
    #[test]
    fn loadup_load_path_reaches_gnu_subdirectories_in_gnu_order() {
        let dir = crate::startup::lisp_dir().canonicalize().unwrap();
        let in_vm = dir.clone();
        let (org, calendar, absent, nested, path) = on_evaluator(move || {
            let mut vm = Vm::new();
            drive(&mut vm, &in_vm).unwrap();
            let mut ask = |s: &str| vm.eval_source(s).and_then(|v| v.display())
                .unwrap_or_else(|e| format!("lookup error: {e}"));
            let found = (ask("(locate-file-internal \"org\" load-path '(\".elc\" \".el\"))"), ask("(locate-file-internal \"calendar\" load-path '(\".elc\" \".el\"))"),
                         ask("(list (locate-file-internal \"xterm\" load-path '(\".elc\" \".el\")) (locate-file-internal \"analyze\" load-path '(\".elc\" \".el\")) (locate-file-internal \"latin-pre\" load-path '(\".elc\" \".el\")))"),
                         ask("(locate-file-internal \"semantic/analyze\" load-path '(\".elc\" \".el\"))"));
            (found.0, found.1, found.2, found.3, load_path_of(&mut vm))
        });
        let quoted = |rel: &str| format!("{:?}", dir.join(rel).to_string_lossy());
        assert_eq!(org, quoted("org/org.el"), "Org is not findable after loadup: load-path {path:?}");
        assert_eq!(calendar, quoted("calendar/calendar.el"), "{path:?}");
        // GNU: term/ is excluded by update-subdirs; cedet/ and leim/ get no
        // subdirs.el of their own (SUBDIRS_SUBDIRS, lisp/Makefile.in:131),
        // so cedet/semantic/ and leim/quail/ are not searched by bare name,
        // only through their parent (GNU -Q: nil nil nil, then the file).
        assert_eq!(absent, "(nil nil nil)", "{path:?}");
        assert_eq!(nested, quoted("cedet/semantic/analyze.el"), "{path:?}");
        let mut expected = vec![dir.to_string_lossy().into_owned()];
        expected.extend(GNU_SUBDIRS.iter().chain(LOADUP_SUBDIRS.iter()).map(|s| dir.join(s).to_string_lossy().into_owned()));
        assert_eq!(path.len(), 31, "{path:#?}");
        assert_eq!(path, expected, "load-path after loadup (a directory outside the GNU pin in lisp/ changes it too)");
    }

/// The text `update-subdirs` writes around its `$subdirs` string.
fn subdirs_el_text(list: &str) -> String {
    format!(";; In load-path, after this directory should come  -*- lexical-binding: t -*-\n;; certain of its subdirectories.  Here we specify them.\n(normal-top-level-add-to-load-path '({list}))\n;; Local Variables:\n;; version-control: never\n;; no-byte-compile: t\n;; no-update-autoloads: t\n;; End:\n")
}

    /// A fixture on which GNU's own `build-aux/update-subdirs` (pin
    /// 45b0f7699df, LC_ALL=C) wrote `lisp/subdirs.el` and `lisp/b/subdirs.el`
    /// and nothing else; the expected texts are its output, byte for byte.
    fn subdirs_fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("romacs-subdirs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lisp = root.join("gnu-lisp");
        for d in ["obsolete", "term", "RCS", "CVS", "Old", "=x", "x~", "a.orig", "b.rej", "foo.el", "foo.elc", ".hidden",
                  "alpha", "mh-e", "zeta", "empty", "b/c", "cedet/semantic", "leim/quail"] {
            std::fs::create_dir_all(lisp.join(d)).unwrap();
        }
        std::fs::write(lisp.join("zfile"), "").unwrap();
        std::os::unix::fs::symlink(lisp.join("alpha"), lisp.join("link")).unwrap();
        lisp.canonicalize().unwrap()
    }

    /// C-locale glob order is byte order, not case-folded or punctuation-ignored
    /// collation. Prepending each name reverses that order in `subdirs.el`.
    #[test]
    fn update_subdirs_uses_c_locale_byte_order() {
        let dir = std::env::temp_dir().join(format!("romacs-subdirs-c-order-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["Zed", "abc", "a-c", "ab"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        let actual = update_subdirs(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(actual, SubdirsEl::File {
            list: "\"abc\" \"ab\" \"a-c\" \"Zed\" ".into(),
            names: vec!["abc".into(), "ab".into(), "a-c".into(), "Zed".into()],
        }, "C-locale byte order must determine subdirs text and load-path precedence");
    }

    #[test]
    fn update_subdirs_writes_what_gnus_script_writes() {
        let lisp = subdirs_fixture("script");
        let text = |dir: &Path| match update_subdirs(dir).unwrap() {
            SubdirsEl::File { list, .. } => Some(subdirs_el_text(&list)),
            SubdirsEl::None => None,
            other => panic!("{other:?}"),
        };
        let line3 = |t: String| t.lines().nth(2).unwrap().to_owned();
        assert_eq!(text(&lisp).map(line3).as_deref(),
                   Some(r#"(normal-top-level-add-to-load-path '("zeta" "mh-e" "link" "leim" "empty" "cedet" "b" "alpha"  "obsolete"))"#));
        assert_eq!(text(&lisp.join("b")).map(line3).as_deref(), Some(r#"(normal-top-level-add-to-load-path '("c" ))"#));
        assert_eq!(text(&lisp.join("empty")), None);
        // The script would write one in cedet/, but GNU's Makefile never runs it there.
        assert_eq!(update_subdirs(&lisp.join("cedet")).unwrap(), SubdirsEl::File { list: "\"semantic\" ".into(), names: vec!["semantic".into()] });
        assert_eq!((gnu_subdirs_el(&lisp, &lisp.join("cedet")), gnu_subdirs_el(&lisp, &lisp.join("leim"))), (SubdirsEl::None, SubdirsEl::None));
        assert_eq!(gnu_subdirs_el(&lisp, &lisp.parent().unwrap().join("elsewhere")), SubdirsEl::None);
        std::fs::remove_dir_all(lisp.parent().unwrap()).unwrap();
        // This tree's lisp/: GNU's script output on the pin's directories
        // (sha256 c706f2735d0d95e1b5834cabed47c9815ef07632a8bf1021afa6945081f3cb23).
        let ours = text(&crate::startup::lisp_dir().canonicalize().unwrap()).unwrap();
        assert_eq!(ours, concat!(
            ";; In load-path, after this directory should come  -*- lexical-binding: t -*-\n",
            ";; certain of its subdirectories.  Here we specify them.\n",
            r#"(normal-top-level-add-to-load-path '("vc" "use-package" "url" "textmodes" "progmodes" "play" "org" "nxml" "net" "mh-e" "mail" "leim" "language" "international" "image" "gnus" "eshell" "erc" "emulation" "emacs-lisp" "cedet" "calendar" "calc"  "obsolete"))"#, "\n",
            ";; Local Variables:\n;; version-control: never\n;; no-byte-compile: t\n;; no-update-autoloads: t\n;; End:\n"));
    }

    /// GNU 32.0.50's own loop and `normal-top-level-add-to-load-path`, run
    /// with `load-path` (b lisp alpha) over the fixture's script-written
    /// files, left `b b/c b/c . zeta mh-e link leim empty cedet b alpha
    /// obsolete alpha`: b/c twice (the second `b` splices after the FIRST),
    /// spliced entries visited in turn, and no file outside lisp/.
    #[test]
    fn apply_subdirs_splices_as_gnus_startup_loop_does() {
        let lisp = subdirs_fixture("splice");
        let at = |rel: &str| if rel == "." { lisp.to_string_lossy().into_owned() } else { lisp.join(rel).to_string_lossy().into_owned() };
        let (path, applied, skipped) = apply_subdirs(vec![at("b"), at("."), at("alpha")], &lisp);
        assert!(skipped.is_empty(), "{skipped:?}");
        let gnu: Vec<String> = "b b/c b/c . zeta mh-e link leim empty cedet b alpha obsolete alpha".split(' ').map(at).collect();
        assert_eq!(path, gnu);
        assert_eq!(applied, [at("b"), at("."), at("b")]);
        // Absent parent: GNU's loop stops at the last element and splices after it.
        let mut tail = vec!["/x".to_owned(), "/y".to_owned()];
        assert_eq!(add_to_load_path(&mut tail, "/z/", &["d".to_owned()]), Some(1));
        assert_eq!(tail, ["/x", "/y", "/z/d"]);
        assert_eq!(add_to_load_path(&mut vec![], "/z", &["d".to_owned()]), None);
        std::fs::remove_dir_all(lisp.parent().unwrap()).unwrap();
    }

    /// Symlinked directories: GNU's Makefile finds directories with `find
    /// -type d` (lisp/Makefile.in:121), which does not follow symlinks, so a
    /// file exists behind `alias -> cedet` only if one exists in `cedet`
    /// (none: excluded), and behind `balias -> b` it is b's. GNU 32.0.50,
    /// files written by the script on that `find` set minus cedet%/leim%,
    /// then startup.el's loop from `(lisp)`: `. cedet balias balias/c b b/c
    /// alias`, and bare `analyze` (in cedet/semantic/) unfindable.
    #[test]
    fn apply_subdirs_follows_gnus_build_through_symlinks() {
        let root = std::env::temp_dir().join(format!("romacs-subdirs-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("gnu-lisp/b/c")).unwrap();
        std::fs::create_dir_all(root.join("gnu-lisp/cedet/semantic")).unwrap();
        std::os::unix::fs::symlink("cedet", root.join("gnu-lisp/alias")).unwrap();
        std::os::unix::fs::symlink("b", root.join("gnu-lisp/balias")).unwrap();
        let lisp = root.join("gnu-lisp").canonicalize().unwrap();
        let at = |rel: &str| if rel == "." { lisp.to_string_lossy().into_owned() } else { lisp.join(rel).to_string_lossy().into_owned() };
        let (path, _, _) = apply_subdirs(vec![at(".")], &lisp);
        std::fs::remove_dir_all(&root).unwrap();
        let gnu: Vec<String> = ". cedet balias balias/c b b/c alias".split(' ').map(at).collect();
        assert_eq!(path, gnu, "a symlinked directory is not GNU's build-time directory set");
    }

    /// Outside `portable`'s domain GNU does what the port does not model. On
    /// script-written files GNU 32.0.50 added `/root` for a directory `~root`
    /// and `<lisp>/az` for `a\z`, read a `"` in a name as the end of the
    /// string and so applied NOTHING of that file, and keeps non-UTF-8 bytes;
    /// lisp/Makefile.in word-splits a path with whitespace, so no file is
    /// written there. The port applies none of such a directory's file and
    /// reports it: never a directory GNU would not add, never a silent omission.
    #[test]
    fn apply_subdirs_applies_nothing_outside_its_domain_and_says_so() {
        use std::os::unix::ffi::OsStrExt;
        let root = std::env::temp_dir().join(format!("romacs-subdirs-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cases: [(&str, &[&[u8]]); 4] = [
            ("rewritten", &[b"ok", b"~root", b"a\\z"]),
            ("quote", &[b"ok", b"q\"x"]),
            ("bytes", &[b"ok", b"bad\xff"]),
            ("space", &[b"ok", b"has space/child"]),
        ];
        let mut results = vec![];
        for (case, dirs) in cases {
            let lisp = root.join(case).join("gnu-lisp");
            for d in dirs { std::fs::create_dir_all(lisp.join(std::ffi::OsStr::from_bytes(d))).unwrap(); }
            let lisp = lisp.canonicalize().unwrap();
            let top = lisp.to_string_lossy().into_owned();
            let (path, applied, skipped) = apply_subdirs(vec![top.clone()], &lisp);
            results.push((case, path == [top.clone()], applied.is_empty(), skipped.iter().map(|(d, why)| (d == &top, why.clone())).collect::<Vec<_>>()));
        }
        // The space case's inner path, reached as a load-path entry itself.
        let lisp = root.join("space/gnu-lisp").canonicalize().unwrap();
        let inner = lisp.join("has space").to_string_lossy().into_owned();
        let (path, _, skipped) = apply_subdirs(vec![lisp.to_string_lossy().into_owned(), inner.clone()], &lisp);
        std::fs::remove_dir_all(&root).unwrap();
        for (case, unchanged, none_applied, skipped) in &results {
            assert!(*unchanged && *none_applied, "{case}: a directory outside the domain reached load-path: {results:?}");
            assert_eq!(skipped.len(), 1, "{case}: not reported: {results:?}");
            assert!(skipped[0].0 && skipped[0].1.starts_with("subdirectory names outside the reproduced domain: "), "{case}: {skipped:?}");
        }
        assert_eq!(results[0].3[0].1, "subdirectory names outside the reproduced domain: a\\z, ~root");
        assert_eq!(path.len(), 2, "{path:?}");
        assert_eq!(skipped.last().map(|s| (s.0.as_str(), s.1.as_str())), Some((inner.as_str(), "directory has space is outside the reproduced domain")));
    }

    /// The text scan finds exactly the `load` calls GNU's reader would: count
    /// them structurally in the VM's own read of each top-level form of this
    /// tree's loadup.el, and check every span reads as one whole form.
    #[test]
    fn load_sites_agree_with_the_reader_on_loadup_el() {
        let text = std::fs::read_to_string(crate::startup::lisp_dir().join("loadup.el")).unwrap();
        let (spans, sites) = load_sites(&text).unwrap();
        let counts = on_evaluator(move || {
            let mut vm = Vm::new();
            vm.install_lisp_prelude().unwrap();
            vm.eval_source("(defun romacs-count-loads (f) (if (consp f) (+ (if (eq (car f) 'load) 1 0) (romacs-count-loads (car f)) (romacs-count-loads (cdr f))) 0))").unwrap();
            spans.iter().map(|&(start, end, _)| {
                let form = &text[start..end];
                let (_, used) = vm.read_source(form).unwrap().unwrap();
                assert_eq!(used, form.len(), "span is not one form: {form}");
                let v = vm.eval_source(&format!("(romacs-count-loads (quote {form}))")).unwrap();
                v.display().unwrap().parse::<usize>().unwrap()
            }).collect::<Vec<_>>()
        });
        let per_form: Vec<usize> = (0..counts.len()).map(|k| sites.iter().filter(|s| s.form == k).count()).collect();
        assert_eq!(per_form, counts);
        assert!(sites.len() > 100, "{}", sites.len());
    }
}
