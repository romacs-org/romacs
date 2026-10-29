//! Headless editor host for source-loaded Lisp tests and development probes.
use romacs_vm::Vm;
use std::io::Read;
use std::path::PathBuf;

/// `--batch RLISP_DIRECTORY [--init-file FILE]' runs the forms on stdin
/// over Romacs's own Lisp; any other `--batch' command line is GNU's
/// ([`run_emacs`]). QUICK is a `-Q' before `--batch'.
pub fn run(args: Vec<String>, quick: bool) -> Result<(), String> {
    let own = matches!(args.as_slice(), [dir] | [dir, _, _] if std::path::Path::new(dir).is_dir())
        && (args.len() == 1 || args[1] == "--init-file");
    if !own { return run_emacs(args, quick); }
    let init = args.get(2).map(PathBuf::from);
    let root = PathBuf::from(&args[0]).canonicalize().map_err(|error| error.to_string())?;
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source).map_err(|error| error.to_string())?;
    if source.trim().is_empty() { return Err("batch input is empty".into()); }
    super::spawn_evaluator(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1000, 700, 20, 10)?;
        super::startup::initialize_files_from_sources(&mut vm, &[], &root)?;
        if let Some(path) = init {
            if !super::user_init::load(&mut vm, &path)? {
                return Err(format!("Explicit init file does not exist: {}", path.display()));
            }
        }
        let value = vm.eval_source(&source)?.display()?;
        println!("{value}");
        vm.shutdown_background_processes()
    })?.join().map_err(|_| "batch evaluator panicked".to_owned())?
}

/// GNU's `emacs --batch ARGS': the GNU-loadup chain `--loadup' shares with
/// the batch package runner, then startup.el's `command-line' as a batch
/// session takes it: the leading options it consumes, no init file and no
/// packages (startup.el:1367), then `command-line-1' over the rest (`-L',
/// `-l', `-f', `--eval' and the others), then `(kill-emacs t)'
/// (startup.el:1748-1751). An error that reaches top level goes to
/// `command-error-function' as keyboard.c cmd_error sends it, whose batch
/// default prints it and ends the session with status 255.
fn run_emacs(args: Vec<String>, quick: bool) -> Result<(), String> {
    let (quick, args) = command_line_options(args, quick);
    super::spawn_evaluator(move || -> Result<(), String> {
        let mut vm = Vm::new();
        vm.set_session_host(Some(Box::new(|code| {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(code)
        })));
        // GNU batch runs on its initial terminal frame: 80 columns by 25
        // lines of 1x1 characters (DIVERGENCES.md VM2-WIN5).
        vm.set_frame_geometry(80, 25, 1, 1)?;
        super::startup::drive_loadup(&mut vm, &super::startup::lisp_dir())?;
        let args: Vec<String> = args.iter().map(|arg| super::startup::lisp_string(arg)).collect();
        // command-line's -Q (startup.el:1412).
        let quick = if quick { "(setq site-run-file nil inhibit-x-resources t)" } else { "" };
        vm.eval_source(&format!(
            "(progn (setq command-line-args (cons (expand-file-name invocation-name invocation-directory) (list {}))
                          command-line-default-directory default-directory)
                    {quick}
                    (condition-case err (command-line-1 (cdr command-line-args))
                      (error (funcall command-error-function err \"\" nil) (kill-emacs -1)))
                    (kill-emacs t))",
            args.join(" ")))?;
        Err("kill-emacs returned".into())
    })?.join().map_err(|_| "batch evaluator panicked".to_owned())?
}

/// The leading options startup.el `command-line' consumes before
/// `command-line-1' (startup.el:1375-1440), as far as a batch session
/// heeds them: whether one was -Q, and the rest.
fn command_line_options(args: Vec<String>, mut quick: bool) -> (bool, Vec<String>) {
    let mut rest = args.into_iter().peekable();
    while let Some(arg) = rest.peek() {
        match arg.as_str() {
            "-Q" | "-quick" | "--quick" => quick = true,
            "-q" | "-no-init-file" | "--no-init-file" | "-no-site-file" | "--no-site-file"
            | "-no-x-resources" | "--no-x-resources" => {}
            _ => break,
        }
        rest.next();
    }
    (quick, rest.collect())
}
