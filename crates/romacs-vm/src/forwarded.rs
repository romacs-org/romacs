//! GNU's built-in variables: symbols FORWARDED to runtime storage.
//!
//! GNU defines 948 variables in C (`DEFVAR_LISP`, `DEFVAR_BOOL`, `DEFVAR_INT`,
//! `DEFVAR_PER_BUFFER`). Such a symbol's redirect is `SYMBOL_FORWARDED`
//! (lisp.h): its value cell holds no Lisp value but a `Lisp_Fwd` naming C
//! storage of one of four kinds, and every variable operation dispatches on it:
//!
//! | GNU (data.c / eval.c)              | VM2                              |
//! |------------------------------------|----------------------------------|
//! | `do_symval_forwarding`             | [`Vm::forwarded_value`]          |
//! | `store_symval_forwarding`          | `Vm::store_forwarded`            |
//! | `set_internal`, FORWARDED arm      | [`Vm::set_forwarded`]            |
//! | `default_value`, FORWARDED arm     | [`Vm::forwarded_default`]        |
//! | `set_default_internal`, FORWARDED  | [`Vm::set_forwarded_default`]    |
//! | `specbind` + `do_specbind`         | [`Vm::bind_forwarded`]           |
//! | `do_one_unbind` (LET/DEFAULT/LOCAL)| [`Vm::unbind_forwarded`]         |
//!
//! VM2's redirect is [`VariableRedirect::Forwarded`], resolved lazily from the
//! symbol NAME like the other redirects, so a code image (which rebuilds slots
//! by name) acquires it too, and an uninterned symbol of the same name never
//! does. The symbol's own value cell stays UNBOUND and is never read.
//!
//! Storage, the counterpart of GNU's C globals and `buffer_defaults`, is
//! [`Forwarded`]: objects (moving heap Values), booleans, integers, and
//! per-buffer defaults (moving heap Values). Per-buffer *values* live in the
//! buffer objects themselves (`PerBuffer`), as in GNU's `struct buffer`.
//!
//! COLLECTOR CONTRACT. `Forwarded::objects` and `Forwarded::defaults` are
//! roots outside every value cell: `gc.rs` marks them with the slots and
//! rewrites them with the slots. A buffer's per-buffer values are marked and
//! rewritten by that buffer's `visit_values`, so they live exactly as long as
//! the buffer. Nothing here retains a borrowed `&Value` across allocation:
//! every accessor copies the Value out. An initial value that needs the heap
//! (a keymap, a string, a list) is materialised on first access, so a bare
//! `Vm::new()` still allocates nothing; UNBOUND marks "not yet materialised"
//! and is never a stored value (`set_internal` refuses to store it).
use super::*;
use crate::prim::Fault;

/// GNU `Lisp_Fwd` kinds (lisp.h). The payload indexes the matching vector
/// of [`Forwarded`]. `Lisp_Fwd_Kboard_Obj` is not modelled (no variable the
/// chain or census reaches needs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fwd {
    Obj(u16),
    Bool(u16),
    Int(u16),
    Buffer(u16),
}

/// GNU `enum Lisp_Fwd_Predicate`, checked by `store_symval_forwarding` for a
/// non-nil value stored into a per-buffer variable. Only the predicates of
/// variables VM2 defines are present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Predicate {
    Integerp,
    Stringp,
    Symbolp,
}

/// EMACS_CONFIGURATION: this host's triple in config.guess's spelling.
const HOST_CONFIGURATION: &str = if cfg!(target_arch = "x86_64") {
    "x86_64-pc-linux-gnu"
} else if cfg!(target_arch = "aarch64") {
    "aarch64-unknown-linux-gnu"
} else {
    "unknown-unknown-linux-gnu"
};

/// C variables that no Rust code reads: GNU's DEFVAR site and initial value,
/// installed as ordinary special variables on first intern. Being plain
/// value cells, they can be let-bound and made buffer-local like any Lisp
/// variable (forwarded objects and integers cannot yet: VM2-CVAR1).
pub(crate) static C_SPECIALS: &[CVariable] = &[
    CVariable { name: "inhibit-x-resources", gnu: "emacs.c:3596", kind: Kind::Bool(false) },
    // VM2 compiles every form lexically, so the variable reads t, as in a
    // lexical-binding file; GNU's default is nil (VM2-LEXBIND1).
    CVariable { name: "lexical-binding", gnu: "lread.c:5761", kind: Kind::Lisp(Init::True) },
    // run-window-scroll-functions (window.c:3862) runs it through ordinary
    // lookup; modes make it buffer-local, as hooks are. xdisp.c:38881 nil.
    CVariable { name: "window-scroll-functions", gnu: "xdisp.c:38865", kind: Kind::Lisp(Init::Nil) },
    // mule-cmds.el's reset-language-environment reads it.
    CVariable { name: "default-process-coding-system", gnu: "coding.c:12133", kind: Kind::Lisp(Init::Nil) },
    // DEFVAR_KBOARD; VM2's single keyboard is the initial terminal's (VM2-WIN5).
    CVariable { name: "window-system", gnu: "dispnew.c:7522", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "initial-window-system", gnu: "dispnew.c:7507", kind: Kind::Lisp(Init::Nil) },
    // command-line sets it once initialization is done; package.el reads it.
    CVariable { name: "after-init-time", gnu: "emacs.c:3591", kind: Kind::Lisp(Init::Nil) },
    // C variables that GNU's preloaded Lisp and the Gnosis/emacs-hermes load
    // closure reference, with their DEFVAR site and C initial value. Each is
    // listed only when that value equals pinned GNU's batch value, so no
    // initialization done later in C (init_*) is misrepresented.
    CVariable { name: "after-delete-frame-functions", gnu: "frame.c:7521", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "alter-fullscreen-frames", gnu: "frame.c:7775", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "ambiguous-width-chars", gnu: "character.c:1123", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: None, extras: &[], init: Init::Nil, ranges: &[] })) },
    CVariable { name: "attempt-orderly-shutdown-on-fatal-signal", gnu: "keyboard.c:14401", kind: Kind::Bool(true) },
    CVariable { name: "attempt-stack-overflow-recovery", gnu: "keyboard.c:14392", kind: Kind::Bool(true) },
    CVariable { name: "auto-composition-emoji-eligible-codepoints", gnu: "composite.c:2305", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "auto-composition-mode", gnu: "composite.c:2239", kind: Kind::Lisp(Init::True) },
    CVariable { name: "auto-fill-chars", gnu: "character.c:1107", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: Some("auto-fill-chars"), extras: &[], init: Init::Nil, ranges: &[(32, 32, Init::True), (10, 10, Init::True)] })) },
    CVariable { name: "auto-hscroll-mode", gnu: "xdisp.c:39075", kind: Kind::Lisp(Init::True) },
    CVariable { name: "auto-resize-tab-bars", gnu: "xdisp.c:38905", kind: Kind::Lisp(Init::True) },
    CVariable { name: "auto-resize-tool-bars", gnu: "xdisp.c:38918", kind: Kind::Lisp(Init::True) },
    CVariable { name: "auto-save-interval", gnu: "keyboard.c:13864", kind: Kind::Int(300) },
    CVariable { name: "auto-save-list-file-name", gnu: "fileio.c:6952", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "auto-save-no-message", gnu: "keyboard.c:13869", kind: Kind::Bool(false) },
    CVariable { name: "auto-save-timeout", gnu: "keyboard.c:13873", kind: Kind::Lisp(Init::Fixnum(30)) },
    CVariable { name: "auto-save-visited-file-name", gnu: "fileio.c:6959", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "backtrace-on-error-noninteractive", gnu: "eval.c:4558", kind: Kind::Bool(true) },
    CVariable { name: "baud-rate", gnu: "dispnew.c:7487", kind: Kind::Int(0) },
    CVariable { name: "bidi-inhibit-bpa", gnu: "xdisp.c:39274", kind: Kind::Bool(false) },
    CVariable { name: "blink-cursor-alist", gnu: "xdisp.c:39063", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "char-code-property-alist", gnu: "chartab.c:1386", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "char-script-table", gnu: "character.c:1141", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: Some("char-script-table"), extras: &[Init::Nil], init: Init::Nil, ranges: &[] })) },
    CVariable { name: "char-width-table", gnu: "character.c:1115", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: None, extras: &[], init: Init::Fixnum(1), ranges: &[(0x80, 0x9f, Init::Fixnum(4)), (0x3fff80, 0x3fffff, Init::Fixnum(4))] })) },
    CVariable { name: "clone-indirect-buffer-hook", gnu: "buffer.c:6027", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "coding-system-require-warning", gnu: "coding.c:12166", kind: Kind::Bool(false) },
    CVariable { name: "combine-after-change-calls", gnu: "insdel.c:2571", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "command-debug-status", gnu: "callint.c:892", kind: Kind::Lisp(Init::Nil) },
    // minibuffer.el's minibuffer-error-initialize makes it buffer-local.
    CVariable { name: "command-error-function", gnu: "keyboard.c:14322", kind: Kind::Lisp(Init::Symbol("command-error-default-function")) },
    CVariable { name: "comment-end-can-be-escaped", gnu: "syntax.c:3816", kind: Kind::Bool(false) },
    CVariable { name: "comment-use-syntax-ppss", gnu: "syntax.c:3751", kind: Kind::Lisp(Init::True) },
    CVariable { name: "compose-chars-after-function", gnu: "composite.c:2223", kind: Kind::Lisp(Init::Symbol("compose-chars-after")) },
    CVariable { name: "composition-break-at-point", gnu: "xdisp.c:39499", kind: Kind::Bool(false) },
    CVariable { name: "composition-function-table", gnu: "composite.c:2270", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: None, extras: &[], init: Init::Nil, ranges: &[] })) },
    CVariable { name: "current-iso639-language", gnu: "charset.c:2430", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "current-key-remap-sequence", gnu: "keyboard.c:14482", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "cursor-in-echo-area", gnu: "dispnew.c:7538", kind: Kind::Bool(false) },
    CVariable { name: "debug-on-event", gnu: "keyboard.c:14381", kind: Kind::Lisp(Init::Symbol("sigusr2")) },
    CVariable { name: "debug-on-next-call", gnu: "eval.c:4515", kind: Kind::Bool(false) },
    CVariable { name: "debug-on-quit", gnu: "eval.c:4510", kind: Kind::Bool(false) },
    CVariable { name: "debug-on-signal", gnu: "eval.c:4552", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "debugger-may-continue", gnu: "eval.c:4523", kind: Kind::Bool(true) },
    CVariable { name: "default-frame-alist", gnu: "frame.c:7428", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "default-frame-scroll-bars", gnu: "frame.c:7453", kind: Kind::Lisp(Init::Symbol("right")) },
    CVariable { name: "default-minibuffer-frame", gnu: "frame.c:7557", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "defining-kbd-macro", gnu: "macros.c:427", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "delayed-warnings-list", gnu: "keyboard.c:14239", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "delete-auto-save-files", gnu: "buffer.c:6016", kind: Kind::Bool(true) },
    CVariable { name: "delete-by-moving-to-trash", gnu: "fileio.c:6991", kind: Kind::Bool(false) },
    CVariable { name: "delete-exited-processes", gnu: "process.c:8933", kind: Kind::Bool(true) },
    CVariable { name: "delete-frame-choose-selected", gnu: "frame.c:7798", kind: Kind::Lisp(Init::Symbol("mru")) },
    CVariable { name: "delete-frame-functions", gnu: "frame.c:7509", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "disable-inhibit-text-conversion", gnu: "keyboard.c:14475", kind: Kind::Bool(false) },
    CVariable { name: "disable-point-adjustment", gnu: "keyboard.c:14289", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "display-fill-column-indicator", gnu: "xdisp.c:39218", kind: Kind::Bool(false) },
    CVariable { name: "display-fill-column-indicator-character", gnu: "xdisp.c:39237", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "display-fill-column-indicator-column", gnu: "xdisp.c:39227", kind: Kind::Lisp(Init::True) },
    CVariable { name: "display-hourglass", gnu: "xdisp.c:39312", kind: Kind::Bool(true) },
    CVariable { name: "display-line-numbers", gnu: "xdisp.c:39155", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "display-line-numbers-current-absolute", gnu: "xdisp.c:39196", kind: Kind::Lisp(Init::True) },
    CVariable { name: "display-line-numbers-major-tick", gnu: "xdisp.c:39246", kind: Kind::Int(0) },
    CVariable { name: "display-line-numbers-minor-tick", gnu: "xdisp.c:39253", kind: Kind::Int(0) },
    CVariable { name: "display-line-numbers-offset", gnu: "xdisp.c:39209", kind: Kind::Int(0) },
    CVariable { name: "display-line-numbers-widen", gnu: "xdisp.c:39203", kind: Kind::Bool(false) },
    CVariable { name: "display-line-numbers-width", gnu: "xdisp.c:39186", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "display-raw-bytes-as-hex", gnu: "xdisp.c:39451", kind: Kind::Bool(false) },
    CVariable { name: "double-click-fuzz", gnu: "keyboard.c:13909", kind: Kind::Int(3) },
    CVariable { name: "double-click-time", gnu: "keyboard.c:13899", kind: Kind::Lisp(Init::Fixnum(500)) },
    CVariable { name: "echo-keystrokes", gnu: "keyboard.c:13880", kind: Kind::Lisp(Init::Fixnum(1)) },
    CVariable { name: "echo-keystrokes-help", gnu: "keyboard.c:13886", kind: Kind::Bool(true) },
    CVariable { name: "enable-character-translation", gnu: "coding.c:12111", kind: Kind::Lisp(Init::True) },
    CVariable { name: "enable-disabled-menus-and-buttons", gnu: "keyboard.c:14342", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "eol-mnemonic-dos", gnu: "coding.c:12096", kind: Kind::Lisp(Init::Str("\\")) },
    CVariable { name: "eol-mnemonic-mac", gnu: "coding.c:12101", kind: Kind::Lisp(Init::Str("/")) },
    CVariable { name: "expose-hidden-buffer", gnu: "frame.c:7750", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "face-remapping-alist", gnu: "xfaces.c:7718", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "fast-but-imprecise-scrolling", gnu: "window.c:9586", kind: Kind::Bool(false) },
    CVariable { name: "fast-read-process-output", gnu: "process.c:8997", kind: Kind::Bool(true) },
    CVariable { name: "file-name-coding-system", gnu: "fileio.c:6785", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "focus-follows-mouse", gnu: "frame.c:7579", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "font-use-system-font", gnu: "xsettings.c:1398", kind: Kind::Bool(false) },
    CVariable { name: "fontification-functions", gnu: "xdisp.c:39011", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "force-load-messages", gnu: "lread.c:5746", kind: Kind::Bool(false) },
    CVariable { name: "forward-comment-function", gnu: "syntax.c:3822", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "frame-inhibit-implied-resize", gnu: "frame.c:7638", kind: Kind::Lisp(Init::True) },
    CVariable { name: "frame-resize-pixelwise", gnu: "frame.c:7625", kind: Kind::Bool(false) },
    CVariable { name: "frame-size-history", gnu: "frame.c:7695", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "garbage-collection-messages", gnu: "alloc.c:7495", kind: Kind::Bool(false) },
    CVariable { name: "gc-cons-percentage", gnu: "alloc.c:7448", kind: Kind::Lisp(Init::Float(0.1)) },
    CVariable { name: "gc-cons-threshold", gnu: "alloc.c:7434", kind: Kind::Int(800000) },
    CVariable { name: "global-mode-string", gnu: "xdisp.c:38701", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "glyph-table", gnu: "dispnew.c:7549", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "glyphless-char-display", gnu: "xdisp.c:39349", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: Some("glyphless-char-display"), extras: &[Init::Symbol("empty-box")], init: Init::Nil, ranges: &[] })) },
    CVariable { name: "help-form", gnu: "keyboard.c:13962", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "highlight-nonselected-windows", gnu: "xdisp.c:38819", kind: Kind::Bool(false) },
    CVariable { name: "hourglass-delay", gnu: "xdisp.c:39318", kind: Kind::Lisp(Init::Fixnum(1)) },
    CVariable { name: "hscroll-margin", gnu: "xdisp.c:39085", kind: Kind::Int(5) },
    CVariable { name: "hscroll-step", gnu: "xdisp.c:39090", kind: Kind::Lisp(Init::Fixnum(0)) },
    CVariable { name: "iconify-child-frame", gnu: "frame.c:7735", kind: Kind::Lisp(Init::Symbol("iconify-top-level")) },
    CVariable { name: "image-scaling-factor", gnu: "image.c:13779", kind: Kind::Lisp(Init::Symbol("auto")) },
    CVariable { name: "indent-tabs-mode", gnu: "indent.c:2579", kind: Kind::Bool(true) },
    CVariable { name: "inherit-process-coding-system", gnu: "coding.c:12023", kind: Kind::Bool(false) },
    CVariable { name: "inhibit--record-char", gnu: "keyboard.c:14454", kind: Kind::Bool(false) },
    CVariable { name: "inhibit-debugger", gnu: "eval.c:4479", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "inhibit-iso-escape-detection", gnu: "coding.c:12175", kind: Kind::Bool(false) },
    CVariable { name: "inhibit-message", gnu: "xdisp.c:38425", kind: Kind::Bool(false) },
    CVariable { name: "inhibit-null-byte-detection", gnu: "coding.c:12203", kind: Kind::Bool(false) },
    CVariable { name: "inhibit-quit", gnu: "eval.c:4449", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "inhibit-redisplay", gnu: "xdisp.c:38696", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "input-method-function", gnu: "keyboard.c:14255", kind: Kind::Lisp(Init::Symbol("list")) },
    CVariable { name: "input-method-previous-message", gnu: "keyboard.c:14277", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "installation-directory", gnu: "emacs.c:3564", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "internal--daemon-sockname", gnu: "process.c:8984", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "interrupt-process-functions", gnu: "process.c:8970", kind: Kind::Lisp(Init::List(&[Init::Symbol("internal-default-interrupt-process")])) },
    CVariable { name: "inverse-video", gnu: "dispnew.c:7492", kind: Kind::Bool(false) },
    CVariable { name: "kbd-macro-termination-hook", gnu: "macros.c:414", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "kill-buffer-delete-auto-save-files", gnu: "buffer.c:6009", kind: Kind::Bool(false) },
    CVariable { name: "kill-emacs-hook", gnu: "emacs.c:3537", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-event-device", gnu: "keyboard.c:13936", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-event-frame", gnu: "keyboard.c:13931", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-input-event", gnu: "keyboard.c:13773", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-kbd-macro", gnu: "macros.c:442", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-nonmenu-event", gnu: "keyboard.c:13767", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-prefix-arg", gnu: "callint.c:872", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "last-repeatable-command", gnu: "keyboard.c:13826", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "line-number-display-limit", gnu: "xdisp.c:38806", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "line-number-display-limit-width", gnu: "xdisp.c:38812", kind: Kind::Int(200) },
    CVariable { name: "line-prefix", gnu: "xdisp.c:39142", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "load-dangerous-libraries", gnu: "lread.c:5739", kind: Kind::Bool(false) },
    CVariable { name: "load-path-filter-function", gnu: "lread.c:5808", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "load-read-function", gnu: "lread.c:5697", kind: Kind::Lisp(Init::Symbol("read")) },
    CVariable { name: "lread--unescaped-character-literals", gnu: "lread.c:5775", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "make-cursor-line-fully-visible", gnu: "xdisp.c:38931", kind: Kind::Lisp(Init::True) },
    CVariable { name: "make-pointer-invisible", gnu: "frame.c:7496", kind: Kind::Lisp(Init::True) },
    CVariable { name: "max-mini-window-height", gnu: "xdisp.c:39037", kind: Kind::Lisp(Init::Float(0.25)) },
    CVariable { name: "maximum-scroll-margin", gnu: "xdisp.c:38751", kind: Kind::Lisp(Init::Float(0.25)) },
    CVariable { name: "memory-full", gnu: "alloc.c:7514", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "menu-prompting", gnu: "keyboard.c:14001", kind: Kind::Bool(true) },
    CVariable { name: "menu-updating-frame", gnu: "xdisp.c:39120", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "message-truncate-lines", gnu: "xdisp.c:39108", kind: Kind::Bool(false) },
    CVariable { name: "messages-buffer-name", gnu: "xdisp.c:38597", kind: Kind::Lisp(Init::Str("*Messages*")) },
    CVariable { name: "minibuffer-auto-raise", gnu: "minibuf.c:2665", kind: Kind::Bool(false) },
    CVariable { name: "minibuffer-completing-file-name", gnu: "minibuf.c:2641", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "minibuffer-completion-confirm", gnu: "minibuf.c:2630", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "minibuffer-completion-predicate", gnu: "minibuf.c:2626", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "minibuffer-completion-table", gnu: "minibuf.c:2611", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "minibuffer-follows-selected-frame", gnu: "minibuf.c:2549", kind: Kind::Lisp(Init::True) },
    CVariable { name: "minibuffer-help-form", gnu: "minibuf.c:2646", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "minibuffer-message-timeout", gnu: "keyboard.c:14310", kind: Kind::Lisp(Init::Fixnum(2)) },
    CVariable { name: "minibuffer-scroll-window", gnu: "window.c:9274", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "mode-line-compact", gnu: "xdisp.c:38645", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "mode-line-in-non-selected-windows", gnu: "window.c:9278", kind: Kind::Bool(true) },
    CVariable { name: "mouse-autoselect-window", gnu: "xdisp.c:38883", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "mouse-fine-grained-tracking", gnu: "xdisp.c:39457", kind: Kind::Bool(false) },
    CVariable { name: "mouse-highlight", gnu: "frame.c:7488", kind: Kind::Lisp(Init::True) },
    CVariable { name: "mouse-leave-buffer-hook", gnu: "callint.c:906", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "mouse-prefer-closest-glyph", gnu: "dispnew.c:7541", kind: Kind::Bool(false) },
    CVariable { name: "move-frame-functions", gnu: "frame.c:7504", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "movemail-program-name", gnu: "callproc.c:2204", kind: Kind::Lisp(Init::Str("movemail")) },
    CVariable { name: "multibyte-syntax-as-symbol", gnu: "syntax.c:3788", kind: Kind::Bool(false) },
    CVariable { name: "multiple-terminals-merge-keyboards", gnu: "keyboard.c:14491", kind: Kind::Bool(false) },
    CVariable { name: "native-comp-jit-compilation", gnu: "comp.c:5597", kind: Kind::Bool(true) },
    CVariable { name: "network-coding-system-alist", gnu: "coding.c:12067", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "next-screen-context-lines", gnu: "window.c:9300", kind: Kind::Int(2) },
    CVariable { name: "next-selection-coding-system", gnu: "w32select.c:1330", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "no-redraw-on-reenter", gnu: "dispnew.c:7501", kind: Kind::Bool(false) },
    CVariable { name: "nobreak-char-display", gnu: "xdisp.c:38653", kind: Kind::Lisp(Init::True) },
    CVariable { name: "open-paren-in-column-0-is-defun-start", gnu: "syntax.c:3792", kind: Kind::Bool(true) },
    CVariable { name: "other-window-scroll-buffer", gnu: "window.c:9284", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "overflow-newline-into-fringe", gnu: "fringe.c:1770", kind: Kind::Lisp(Init::True) },
    CVariable { name: "fringe-bitmaps", gnu: "fringe.c:1779", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "overlay-arrow-position", gnu: "xdisp.c:38705", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "overline-margin", gnu: "xdisp.c:39296", kind: Kind::Int(2) },
    CVariable { name: "overriding-terminal-local-map", gnu: "keyboard.c:14128", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "parse-sexp-ignore-comments", gnu: "syntax.c:3769", kind: Kind::Bool(false) },
    CVariable { name: "pgtk-lost-selection-functions", gnu: "pgtkselect.c:1919", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "pgtk-use-im-context-on-new-connection", gnu: "pgtkim.c:306", kind: Kind::Lisp(Init::True) },
    CVariable { name: "polling-period", gnu: "keyboard.c:13892", kind: Kind::Lisp(Init::Float(2.0)) },
    CVariable { name: "post-command-hook", gnu: "keyboard.c:14061", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "post-select-region-hook", gnu: "keyboard.c:14469", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "pre-command-hook", gnu: "keyboard.c:14045", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "printable-chars", gnu: "character.c:1131", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: None, extras: &[], init: Init::Nil, ranges: &[(32, 126, Init::True), (160, 0x3fff7f, Init::True)] })) },
    CVariable { name: "process-adaptive-read-buffering", gnu: "process.c:8948", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "process-coding-system-alist", gnu: "coding.c:12051", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "process-connection-type", gnu: "process.c:8940", kind: Kind::Lisp(Init::True) },
    CVariable { name: "process-error-pause-time", gnu: "process.c:9004", kind: Kind::Int(1) },
    CVariable { name: "process-prioritize-lower-fds", gnu: "process.c:8960", kind: Kind::Bool(false) },
    CVariable { name: "pure-bytes-used", gnu: "alloc.c:7462", kind: Kind::Int(0) },
    CVariable { name: "quit-flag", gnu: "eval.c:4440", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "read-buffer-completion-ignore-case", gnu: "minibuf.c:2559", kind: Kind::Bool(false) },
    CVariable { name: "read-buffer-function", gnu: "minibuf.c:2544", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "read-circle", gnu: "lread.c:5566", kind: Kind::Lisp(Init::True) },
    CVariable { name: "read-expression-history", gnu: "minibuf.c:2539", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "read-hide-char", gnu: "minibuf.c:2699", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "read-minibuffer-restore-windows", gnu: "minibuf.c:2717", kind: Kind::Bool(true) },
    CVariable { name: "read-process-output-max", gnu: "process.c:8988", kind: Kind::Int(65536) },
    CVariable { name: "read-symbol-shorthands", gnu: "lread.c:5884", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "recenter-redisplay", gnu: "window.c:9484", kind: Kind::Lisp(Init::Symbol("tty")) },
    CVariable { name: "record-all-keys", gnu: "keyboard.c:14462", kind: Kind::Bool(false) },
    CVariable { name: "report-emacs-bug-address", gnu: "emacs.c:3613", kind: Kind::Lisp(Init::Str("bug-gnu-emacs@gnu.org")) },
    CVariable { name: "resize-mini-frames", gnu: "frame.c:7566", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "resize-mini-windows", gnu: "xdisp.c:39045", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "ring-bell-function", gnu: "terminal.c:695", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "saved-region-selection", gnu: "keyboard.c:14363", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "scalable-fonts-allowed", gnu: "xfaces.c:7692", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "script-representative-chars", gnu: "character.c:1149", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "scroll-bar-adjust-thumb-portion", gnu: "frame.c:7467", kind: Kind::Bool(true) },
    CVariable { name: "scroll-conservatively", gnu: "xdisp.c:38731", kind: Kind::Int(0) },
    CVariable { name: "scroll-margin", gnu: "xdisp.c:38745", kind: Kind::Int(0) },
    CVariable { name: "scroll-minibuffer-conservatively", gnu: "xdisp.c:38418", kind: Kind::Bool(true) },
    CVariable { name: "scroll-step", gnu: "xdisp.c:38724", kind: Kind::Int(0) },
    CVariable { name: "search-spaces-regexp", gnu: "search.c:3457", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "select-active-regions", gnu: "keyboard.c:14350", kind: Kind::Lisp(Init::True) },
    CVariable { name: "selection-coding-system", gnu: "w32select.c:1324", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "shared-game-score-directory", gnu: "callproc.c:2134", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "shell-file-name", gnu: "callproc.c:2096", kind: Kind::Lisp(Init::Getenv("SHELL", "/bin/sh")) },
    CVariable { name: "show-trailing-whitespace", gnu: "xdisp.c:38640", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "signal-process-functions", gnu: "process.c:8977", kind: Kind::Lisp(Init::List(&[Init::Symbol("internal-default-signal-process")])) },
    CVariable { name: "special-mirror-table", gnu: "xdisp.c:39379", kind: Kind::Lisp(Init::CharTable(&CharTableInit { purpose: None, extras: &[], init: Init::Nil, ranges: &[] })) },
    CVariable { name: "standard-translation-table-for-decode", gnu: "coding.c:12116", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "standard-translation-table-for-encode", gnu: "coding.c:12121", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "symbols-with-pos-enabled", gnu: "data.c:4386", kind: Kind::Bool(false) },
    CVariable { name: "syntax-propertize--done", gnu: "syntax.c:3778", kind: Kind::Int(-1) },
    CVariable { name: "system-configuration", gnu: "emacs.c:3517", kind: Kind::Lisp(Init::Str(HOST_CONFIGURATION)) },
    CVariable { name: "system-configuration-options", gnu: "emacs.c:3521", kind: Kind::Lisp(Init::Str("")) },
    CVariable { name: "system-key-alist", gnu: "keyboard.c:14170", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "system-messages-locale", gnu: "emacs.c:3573", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "system-time-locale", gnu: "emacs.c:3581", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "system-uses-terminfo", gnu: "term.c:5325", kind: Kind::Bool(true) },
    CVariable { name: "tab-bar--dragging-in-progress", gnu: "xdisp.c:39466", kind: Kind::Bool(false) },
    CVariable { name: "tab-bar-mode", gnu: "frame.c:7537", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "tab-bar-position", gnu: "dispnew.c:7570", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "temp-buffer-show-function", gnu: "window.c:9266", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "throw-on-input", gnu: "keyboard.c:14316", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "timer-list", gnu: "keyboard.c:14247", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "timer-idle-list", gnu: "keyboard.c:14251", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "tool-bar-separator-image-expression", gnu: "keyboard.c:14122", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "tool-bar-style", gnu: "xdisp.c:38992", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "toolkit-theme", gnu: "frame.c:7896", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "tooltip-reuse-hidden-frame", gnu: "frame.c:7712", kind: Kind::Bool(false) },
    CVariable { name: "track-mouse", gnu: "keyboard.c:14157", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "translate-upper-case-key-bindings", gnu: "keyboard.c:14421", kind: Kind::Bool(true) },
    CVariable { name: "translation-hash-table-vector", gnu: "ccl.c:2382", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "translation-table-for-input", gnu: "coding.c:12221", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "translation-table-vector", gnu: "character.c:1100", kind: Kind::Lisp(Init::NilVector(16)) },
    CVariable { name: "treesit-extra-load-path", gnu: "treesit.c:5429", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "treesit-thing-settings", gnu: "treesit.c:5443", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "truncate-partial-width-windows", gnu: "xdisp.c:38768", kind: Kind::Lisp(Init::Fixnum(50)) },
    CVariable { name: "tty-erase-char", gnu: "keyboard.c:13948", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "underline-minimum-offset", gnu: "xdisp.c:39302", kind: Kind::Int(1) },
    CVariable { name: "undo-outer-limit", gnu: "undo.c:459", kind: Kind::Lisp(Init::Fixnum(24000000)) },
    CVariable { name: "undo-strong-limit", gnu: "undo.c:447", kind: Kind::Int(240000) },
    CVariable { name: "unibyte-display-via-language-environment", gnu: "xdisp.c:39025", kind: Kind::Bool(false) },
    CVariable { name: "unicode-category-table", gnu: "character.c:1159", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "unread-post-input-method-events", gnu: "keyboard.c:13786", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "unread-input-method-events", gnu: "keyboard.c:13792", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "use-dialog-box", gnu: "fns.c:6843", kind: Kind::Bool(true) },
    CVariable { name: "use-file-dialog", gnu: "fns.c:6852", kind: Kind::Bool(true) },
    CVariable { name: "use-short-answers", gnu: "fns.c:6860", kind: Kind::Bool(false) },
    CVariable { name: "use-system-tooltips", gnu: "frame.c:7727", kind: Kind::Bool(true) },
    CVariable { name: "user-init-file", gnu: "lread.c:5684", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "values", gnu: "lread.c:5555", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "visible-bell", gnu: "dispnew.c:7496", kind: Kind::Bool(false) },
    CVariable { name: "visible-cursor", gnu: "term.c:5347", kind: Kind::Bool(true) },
    CVariable { name: "void-text-area-pointer", gnu: "xdisp.c:38689", kind: Kind::Lisp(Init::Symbol("arrow")) },
    CVariable { name: "window-buffer-change-functions", gnu: "window.c:9329", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-point-insertion-type", gnu: "window.c:9323", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-restore-killed-buffer-windows", gnu: "window.c:9428", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-selection-change-functions", gnu: "window.c:9367", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-size-change-functions", gnu: "window.c:9348", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-state-change-functions", gnu: "window.c:9381", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "word-wrap-by-category", gnu: "xdisp.c:38789", kind: Kind::Bool(false) },
    CVariable { name: "words-include-escapes", gnu: "syntax.c:3785", kind: Kind::Bool(false) },
    CVariable { name: "wrap-prefix", gnu: "xdisp.c:39129", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "write-region-annotations-so-far", gnu: "fileio.c:6936", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "x-gtk-file-dialog-help-text", gnu: "xfns.c:10478", kind: Kind::Bool(true) },
    CVariable { name: "x-gtk-show-hidden-files", gnu: "xfns.c:10472", kind: Kind::Bool(false) },
    CVariable { name: "x-gtk-use-old-file-dialog", gnu: "xfns.c:10465", kind: Kind::Bool(false) },
    CVariable { name: "x-max-tooltip-size", gnu: "xfns.c:10441", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "x-resource-name", gnu: "frame.c:7397", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "x-stretch-cursor", gnu: "xdisp.c:38633", kind: Kind::Bool(false) },
    CVariable { name: "x-underline-at-descent-line", gnu: "xterm.c:32704", kind: Kind::Bool(false) },
    CVariable { name: "x-use-underline-position-properties", gnu: "xterm.c:32693", kind: Kind::Bool(true) },
    // C default; menu-bar.el sets (help-menu).
    CVariable { name: "menu-bar-final-items", gnu: "keyboard.c:14110", kind: Kind::Lisp(Init::Nil) },
];

/// C variables GNU makes automatically buffer-local at startup
/// (`Fmake_variable_buffer_local` in buffer.c, syntax.c, xdisp.c, ...).
const AUTOMATICALLY_LOCAL: &[&str] = &["syntax-propertize--done", "case-symbols-as-words", "lexical-binding",
    "comment-end-can-be-escaped", "display-fill-column-indicator", "display-fill-column-indicator-character",
    "display-fill-column-indicator-column", "display-line-numbers", "display-line-numbers-offset",
    "display-line-numbers-widen", "display-line-numbers-width", "expose-hidden-buffer", "fontification-functions",
    "line-prefix", "make-window-start-visible", "treesit-language-remap-alist", "wrap-prefix"];

/// A C initial value. Heap shapes are built fresh on first access.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Init {
    Nil,
    True,
    Fixnum(i64),
    Symbol(&'static str),
    Str(&'static str),
    /// make_float (X).
    Float(f64),
    /// build_string (getenv (NAME) ?: FALLBACK), read at startup.
    Getenv(&'static str, &'static str),
    SystemName,
    SourceDirectory,
    /// A proper list of the given elements.
    List(&'static [Init]),
    /// make_nil_vector (N).
    NilVector(usize),
    /// The runtime's etc/ directory: ROMACS_RUNTIME_ROOT/etc, else this
    /// checkout's (GNU's installation-relative data-directory).
    EtcDirectory,
    /// init_cmdargs: the running executable's file name (true) or its
    /// directory, with a final slash (false).
    Invocation(bool),
    /// init_editfns: user-login-name, user-real-login-name or user-full-name.
    UserIdentity(&'static str),
    Cons(&'static Init, &'static Init),
    /// Fmake_char_table (PURPOSE, INIT), then these range writes.
    CharTable(&'static CharTableInit),
}

#[derive(Debug)]
pub(crate) struct CharTableInit {
    pub purpose: Option<&'static str>,
    /// The extra slots' values, as many as PURPOSE's char-table-extra-slots
    /// (realized with the symbol, initialize_case_table_slot).
    pub extras: &'static [Init],
    pub init: Init,
    pub ranges: &'static [(u32, u32, Init)],
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    /// `DEFVAR_LISP`.
    Lisp(Init),
    /// `DEFVAR_BOOL`.
    Bool(bool),
    /// `DEFVAR_INT` (an `intmax_t` in GNU; VM2 has fixnums only).
    Int(i64),
    /// `DEFVAR_PER_BUFFER` with a positive `PER_BUFFER_IDX`: local in a
    /// buffer only once set there.
    PerBuffer { default: Init, predicate: Option<Predicate> },
}

pub(crate) struct CVariable {
    pub name: &'static str,
    /// GNU 32.0.50 source of the DEFVAR (provenance; checked by the tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub gnu: &'static str,
    pub kind: Kind,
}

/// Every built-in variable VM2 defines, and why. GNU has 948; VM2 defines
/// only those the bootstrap chain, the census or the variable harness
/// actually reached (ruling R216: no speculative ports).
pub(crate) static C_VARIABLES: &[CVariable] = &[
    // cl-generic's dispatcher compiler checks whether GNU is being dumped.
    CVariable { name: "purify-flag", gnu: "alloc.c:7487", kind: Kind::Lisp(Init::Nil) },
    // GNU warning text expands command keys while compiling obsolete method calls.
    CVariable { name: "overriding-local-map", gnu: "keyboard.c:14137", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "tab-width", gnu: "buffer.c:5284", kind: Kind::PerBuffer { default: Init::Fixnum(8), predicate: Some(Predicate::Integerp) } },
    CVariable { name: "noninteractive", gnu: "emacs.c:3534", kind: Kind::Bool(false) },
    CVariable { name: "auto-window-vscroll", gnu: "window.c:9296", kind: Kind::Bool(true) },
    CVariable { name: "inhibit-point-motion-hooks", gnu: "textprop.c:2412", kind: Kind::Lisp(Init::True) },
    CVariable { name: "inhibit-modification-hooks", gnu: "insdel.c:2575", kind: Kind::Bool(false) },
    CVariable { name: "overriding-plist-environment", gnu: "fns.c:6809", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "selective-display", gnu: "buffer.c:5475", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // No Lisp sets its default; gnosis's dashboard and source choices set it.
    CVariable { name: "header-line-format", gnu: "buffer.c:5158", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "inhibit-field-text-motion", gnu: "editfns.c:4830", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "buffer-invisibility-spec", gnu: "buffer.c:5849", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    CVariable { name: "overlay-arrow-variable-list", gnu: "xdisp.c:38716", kind: Kind::Lisp(Init::List(&[Init::Symbol("overlay-arrow-position")])) },
    CVariable { name: "frame-internal-parameters", gnu: "frame.c:7764", kind: Kind::Lisp(Init::List(&[Init::Symbol("undeleted"), Init::Symbol("cloned-from"), Init::Symbol("frame-id"), Init::Symbol("name"), Init::Symbol("parent-id"), Init::Symbol("window-id")])) },
    CVariable { name: "minor-mode-map-alist", gnu: "keymap.c:3366", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "max-lisp-eval-depth", gnu: "eval.c:4424", kind: Kind::Int(1600) },
    CVariable { name: "lisp-eval-depth-reserve", gnu: "eval.c:4434", kind: Kind::Int(200) },
    CVariable { name: "current-time-list", gnu: "timefns.c:2110", kind: Kind::Bool(false) },
    // Chain: subr.el form 303 (line 3929) `read-char-from-minibuffer-map'.
    // keymap.c:3364 initialises it with Fmake_sparse_keymap (Qnil).
    CVariable { name: "minibuffer-local-map", gnu: "keymap.c:3362", kind: Kind::Lisp(Init::List(&[Init::Symbol("keymap")])) },
    // Census (first failure of a file at 6eb76ddc, WARM and COLD counts):
    // 19/19 files.
    CVariable { name: "system-type", gnu: "emacs.c:3500", kind: Kind::Lisp(Init::Symbol("gnu/linux")) },
    CVariable { name: "system-name", gnu: "editfns.c:4858", kind: Kind::Lisp(Init::SystemName) },
    // GNU expands "../" against the build-time PATH_DUMPLOADSEARCH's first
    // entry (the source lisp directory). Preserve that build source root,
    // not the runtime working directory or the oracle's separate checkout.
    CVariable { name: "source-directory", gnu: "lread.c:5724", kind: Kind::Lisp(Init::SourceDirectory) },
    // 3/3 files; dispnew.c initialises it to nil.
    CVariable { name: "standard-display-table", gnu: "dispnew.c:7560", kind: Kind::Lisp(Init::Nil) },
    // 2/1 files; emacs.c:3611 builds it from the version VM2 is measured
    // against (GNU 32.0.50).
    CVariable { name: "emacs-version", gnu: "emacs.c:3604", kind: Kind::Lisp(Init::Str("32.0.50")) },
    // 1/0 files; keyboard.c:13955 XSETINT (Vhelp_char, Ctl ('H')).
    CVariable { name: "help-char", gnu: "keyboard.c:13951", kind: Kind::Lisp(Init::Fixnum(8)) },
    // 1/0 files; window.c: list1 (Fcons (Qclone_of, Qt)).
    CVariable { name: "window-persistent-parameters", gnu: "window.c:9550", kind: Kind::Lisp(Init::List(&[Init::Cons(&Init::Symbol("clone-of"), &Init::True)])) },
    // 0/1 files; window.c initialises it to nil.
    CVariable { name: "scroll-preserve-screen-position", gnu: "window.c:9304", kind: Kind::Lisp(Init::Nil) },
    // GNU window.el's split-window (window.el:5755-5756, 5843) reads these
    // three window.c variables on every split (prims/window.rs). C values:
    // window.c:9548 Qwindow_size, :9510 nil, :9584 false.
    CVariable { name: "window-combination-limit", gnu: "window.c:9512", kind: Kind::Lisp(Init::Symbol("window-size")) },
    CVariable { name: "window-combination-resize", gnu: "window.c:9491", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-resize-pixelwise", gnu: "window.c:9575", kind: Kind::Bool(false) },
    // Variable harness (R216 task 2): a DEFVAR_BOOL that subr.el reads.
    // get-load-suffixes (lread.c:961) reads both; it is reached through
    // GNU's Lisp locate-library (subr.el:3440) once lazy bodies replaced
    // VM2's native one. GNU 32.0.50 here is built with modules
    // (MODULES_SUFFIX ".so"), so ".so" leads load-suffixes. load-file-rep-
    // suffixes is the C value (""); the dumped GNU image adds ".gz" from
    // jka-cmpr-hook.el, which VM2 has not loaded.
    CVariable { name: "load-suffixes", gnu: "lread.c:5580", kind: Kind::Lisp(Init::List(&[Init::Str(".so"), Init::Str(".elc"), Init::Str(".el")])) },
    CVariable { name: "load-file-rep-suffixes", gnu: "lread.c:5616", kind: Kind::Lisp(Init::List(&[Init::Str("")])) },
    // Census (warm): pcase.el form 12 (line 173) reads after-load-alist and
    // form 52 (line 1190) binds current-load-list. Both are nil in C. GNU's
    // C maintains them while it runs (load specbinds current-load-list,
    // LOADHIST_ATTACH conses onto it, provide runs after-load-alist); VM2's
    // loader binds it to (FILE) but attaches no definitions and runs no
    // after-load callbacks (DIVERGENCES.md VM2-LOADHIST1).
    CVariable { name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Lisp(Init::Nil) },
    // `load' binds these as a `let' does, so their watchers see GNU's
    // forwarded restore: `set', then `unlet'.
    CVariable { name: "load-file-name", gnu: "lread.c:5672", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "load-true-file-name", gnu: "lread.c:5680", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "load-in-progress", gnu: "lread.c:5633", kind: Kind::Bool(false) },
    // Backtrace collection/printing reads these native defaults. Source-history
    // merging remains VM2-LOADHIST1; eval-buffer/eval-region are unsupported.
    CVariable { name: "load-history", gnu: "lread.c:5652", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "eval-buffer-list", gnu: "lread.c:5771", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "enable-recursive-minibuffers", gnu: "minibuf.c:2603", kind: Kind::Bool(false) },
    // read-from-minibuffer / read-string (prims/minibuf.rs, lane vm2/minibuf):
    // the C variables read_minibuf and its command loop read. All nil/false
    // in C except history-add-new-input (minibuf.c:2594) and the prompt
    // properties (minibuf.c:2697, list2 (Qread_only, Qt); GNU's faces.el:2914
    // appends `face minibuffer-prompt' from Lisp).
    CVariable { name: "unread-command-events", gnu: "keyboard.c:13776", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "executing-kbd-macro", gnu: "macros.c:435", kind: Kind::Lisp(Init::Nil) },
    // The command loop's Lisp half (rlisp/vm/keyboard.el) advances it per
    // event read from the macro (keyboard.c read_char).
    CVariable { name: "executing-kbd-macro-index", gnu: "macros.c:439", kind: Kind::Int(0) },
    // Counted by the same Lisp half; repeat.el compares num-input-keys.
    CVariable { name: "num-input-keys", gnu: "keyboard.c:13920", kind: Kind::Int(0) },
    CVariable { name: "num-nonmacro-input-events", gnu: "keyboard.c:13926", kind: Kind::Int(0) },
    CVariable { name: "inhibit-interaction", gnu: "minibuf.c:2708", kind: Kind::Bool(false) },
    CVariable { name: "minibuffer-allow-text-properties", gnu: "minibuf.c:2684", kind: Kind::Bool(false) },
    CVariable { name: "minibuffer-prompt-properties", gnu: "minibuf.c:2693", kind: Kind::Lisp(Init::List(&[Init::Symbol("read-only"), Init::True])) },
    CVariable { name: "history-add-new-input", gnu: "minibuf.c:2589", kind: Kind::Lisp(Init::True) },
    CVariable { name: "minibuffer-setup-hook", gnu: "minibuf.c:2564", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "debug-on-error", gnu: "eval.c:4485", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "debugger-stack-frame-as-list", gnu: "eval.c:4529", kind: Kind::Bool(false) },
    CVariable { name: "signal-hook-function", gnu: "eval.c:4487", kind: Kind::Lisp(Init::Nil) },
    // Variable harness (R216 task 2): a DEFVAR_INT that subr.el reads.
    CVariable { name: "undo-limit", gnu: "undo.c:437", kind: Kind::Int(160_000) },
    // Variable harness (R216 task 2) and DIVERGENCES.md VM2-VARALIAS1: the
    // per-buffer kind. buffer.c:4898 default 70, predicate Qintegerp.
    CVariable { name: "major-mode", gnu: "buffer.c:5242", kind: Kind::PerBuffer { default: Init::Symbol("fundamental-mode"), predicate: Some(Predicate::Symbolp) } },
    CVariable { name: "mode-name", gnu: "buffer.c:5255", kind: Kind::PerBuffer { default: Init::Str("Fundamental"), predicate: None } },
    CVariable { name: "fill-column", gnu: "buffer.c:5259", kind: Kind::PerBuffer { default: Init::Fixnum(70), predicate: Some(Predicate::Integerp) } },
    // Chain: compile.el's easy-mmode docstrings fill through back-to-indentation.
    CVariable { name: "left-margin", gnu: "buffer.c:5279", kind: Kind::PerBuffer { default: Init::Fixnum(0), predicate: Some(Predicate::Integerp) } },
    // Chain: cus-start.el (preloaded) signals for a missing built-in.
    CVariable { name: "abbrev-mode", gnu: "buffer.c:5265", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // self-insert-command (rlisp/vm/prelude.el) reads both.
    CVariable { name: "auto-fill-function", gnu: "buffer.c:5433", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // simple.el's delete-backward-char and self-insert paths read it; GNU
    // checks a non-nil value against its `choice' list (VM2-CVAR-PRED1).
    CVariable { name: "overwrite-mode", gnu: "buffer.c:5492", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "ctl-arrow", gnu: "buffer.c:5294", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    // char-width and string-width read it (indent.c buffer_display_table).
    CVariable { name: "buffer-display-table", gnu: "buffer.c:5502", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "bidi-paragraph-direction", gnu: "buffer.c:5383", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "bidi-display-reordering", gnu: "buffer.c:5332", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    CVariable { name: "bidi-paragraph-start-re", gnu: "buffer.c:5340", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "bidi-paragraph-separate-re", gnu: "buffer.c:5362", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "word-wrap", gnu: "buffer.c:5408", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "selective-display-ellipses", gnu: "buffer.c:5487", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    CVariable { name: "indicate-empty-lines", gnu: "buffer.c:5612", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "indicate-buffer-boundaries", gnu: "buffer.c:5619", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "fringe-indicator-alist", gnu: "buffer.c:5644", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "fringe-cursor-alist", gnu: "buffer.c:5663", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // GNU checks these two against `fraction' on set (VM2-CVAR-PRED1).
    CVariable { name: "scroll-up-aggressively", gnu: "buffer.c:5678", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "scroll-down-aggressively", gnu: "buffer.c:5691", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "cursor-type", gnu: "buffer.c:5905", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    CVariable { name: "line-spacing", gnu: "buffer.c:5928", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "cursor-in-non-selected-windows", gnu: "buffer.c:5939", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    // insert-file-contents / write-region (prims/fileio.rs): the per-buffer
    // C variables those DEFUNs read or set. Their other C variables are
    // plain specials (`Vm::declare_fileio_specials`) so they stay
    // localizable, as add-hook LOCAL needs for the change hooks.
    CVariable { name: "write-region-inhibit-fsync", gnu: "fileio.c:6971", kind: Kind::Bool(true) },
    // Always local and permanent (PER_BUFFER_IDX -1); a new buffer copies
    // the current buffer's (buffer.c:1093 reset_buffer), the first one gets
    // the process directory (buffer.c:4969 init_buffer).
    CVariable { name: "default-directory", gnu: "buffer.c:5426", kind: Kind::PerBuffer { default: Init::Nil, predicate: Some(Predicate::Stringp) } },
    CVariable { name: "buffer-file-name", gnu: "buffer.c:5441", kind: Kind::PerBuffer { default: Init::Nil, predicate: Some(Predicate::Stringp) } },
    CVariable { name: "buffer-file-truename", gnu: "buffer.c:5446", kind: Kind::PerBuffer { default: Init::Nil, predicate: Some(Predicate::Stringp) } },
    CVariable { name: "buffer-read-only", gnu: "buffer.c:5458", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "buffer-file-format", gnu: "buffer.c:5837", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // define-minor-mode records a buffer's enabled minor modes here, and
    // command-completion-default-include-p reads them; nil in a new buffer.
    CVariable { name: "local-minor-modes", gnu: "buffer.c:5249", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // Local only when set; permanent (buffer.c:4811 buffer_permanent_local_flags).
    CVariable { name: "buffer-file-coding-system", gnu: "buffer.c:5314", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "mark-active", gnu: "buffer.c:5795", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // Text edits record undo entries here (undo.c); t disables recording.
    CVariable { name: "buffer-undo-list", gnu: "buffer.c:5743", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // rename-buffer renames a non-visiting buffer's auto-save file.
    CVariable { name: "buffer-auto-save-file-name", gnu: "buffer.c:5452", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    // auto-save reads and resets it; files.el and gnosis read it.
    CVariable { name: "buffer-saved-size", gnu: "buffer.c:5465", kind: Kind::PerBuffer { default: Init::Fixnum(0), predicate: Some(Predicate::Integerp) } },
    CVariable { name: "transient-mark-mode", gnu: "buffer.c:5875", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "mark-even-if-inactive", gnu: "callint.c:898", kind: Kind::Lisp(Init::True) },
    // Heap-valued C variables materialize lazily on first read.
    CVariable { name: "latin-extra-code-table", gnu: "coding.c:12140", kind: Kind::Lisp(Init::NilVector(256)) },
    CVariable { name: "user-full-name", gnu: "editfns.c:4862", kind: Kind::Lisp(Init::UserIdentity("user-full-name")) },
    CVariable { name: "user-login-name", gnu: "editfns.c:4865", kind: Kind::Lisp(Init::UserIdentity("user-login-name")) },
    CVariable { name: "user-real-login-name", gnu: "editfns.c:4869", kind: Kind::Lisp(Init::UserIdentity("user-real-login-name")) },
    CVariable { name: "data-directory", gnu: "callproc.c:2119", kind: Kind::Lisp(Init::EtcDirectory) },
    CVariable { name: "invocation-name", gnu: "emacs.c:3556", kind: Kind::Lisp(Init::Invocation(true)) },
    CVariable { name: "invocation-directory", gnu: "emacs.c:3560", kind: Kind::Lisp(Init::Invocation(false)) },
    CVariable { name: "doc-directory", gnu: "callproc.c:2123", kind: Kind::Lisp(Init::EtcDirectory) },
    CVariable { name: "internal-doc-file-name", gnu: "doc.c:691", kind: Kind::Lisp(Init::Str("DOC")) },
    CVariable { name: "eol-mnemonic-undecided", gnu: "coding.c:12106", kind: Kind::Lisp(Init::Str(":")) },
    CVariable { name: "eol-mnemonic-unix", gnu: "coding.c:12091", kind: Kind::Lisp(Init::Str(":")) },
    CVariable { name: "overlay-arrow-string", gnu: "xdisp.c:38711", kind: Kind::Lisp(Init::Str("=>")) },
    CVariable { name: "yes-or-no-prompt", gnu: "fns.c:6874", kind: Kind::Lisp(Init::Str("(yes or no) ")) },
];

/// C_SPECIALS that GNU leaves bound but not special: lread.c:5559 clears
/// `declared_special' on `values', so a `let' of it is lexical.
pub(crate) const NOT_SPECIAL: &[&str] = &["values"];

/// The runtime storage GNU keeps in C globals and `buffer_defaults`.
pub(crate) struct Forwarded {
    objects: Vec<Value>,
    fundamental_mode_name: Value,
    unbound_symbol: Value,
    bools: Vec<bool>,
    ints: Vec<i64>,
    eval_limit_index: usize,
    /// Per-buffer defaults (`buffer_defaults`).
    defaults: Vec<Value>,
    /// Table index behind each Obj / Buffer payload, for lazy initial values.
    object_vars: Vec<usize>,
    buffer_vars: Vec<usize>,
    by_name: HashMap<&'static str, Fwd>,
}

impl Default for Forwarded {
    fn default() -> Self {
        let mut store = Forwarded {
            objects: vec![],
            fundamental_mode_name: Value::UNBOUND,
            unbound_symbol: Value::UNBOUND,
            bools: vec![],
            ints: vec![],
            eval_limit_index: 0,
            defaults: vec![],
            object_vars: vec![],
            buffer_vars: vec![],
            by_name: HashMap::default(),
        };
        for (index, var) in C_VARIABLES.iter().enumerate() {
            let fwd = match var.kind {
                Kind::Lisp(_) => {
                    store.objects.push(Value::UNBOUND);
                    store.object_vars.push(index);
                    Fwd::Obj(store.objects.len() as u16 - 1)
                }
                Kind::Bool(b) => {
                    store.bools.push(b);
                    Fwd::Bool(store.bools.len() as u16 - 1)
                }
                Kind::Int(n) => {
                    store.ints.push(n);
                    Fwd::Int(store.ints.len() as u16 - 1)
                }
                Kind::PerBuffer { .. } => {
                    store.defaults.push(Value::UNBOUND);
                    store.buffer_vars.push(index);
                    Fwd::Buffer(store.defaults.len() as u16 - 1)
                }
            };
            store.by_name.insert(var.name, fwd);
        }
        let Some(Fwd::Int(i)) = store.lookup("max-lisp-eval-depth") else { unreachable!() };
        store.eval_limit_index = i as usize;
        store
    }
}

impl Forwarded {
    #[inline(always)]
    pub(super) fn eval_limit(&self) -> i64 { self.ints[self.eval_limit_index] }
    /// Direct C-global integer access, without constructing a Lisp Value.
    pub(super) fn integer(&self, name: &str) -> i64 {
        let Some(Fwd::Int(i)) = self.lookup(name) else { panic!("not a forwarded integer: {name}") };
        self.ints[i as usize]
    }
    pub(super) fn set_integer(&mut self, name: &str, value: i64) {
        let Some(Fwd::Int(i)) = self.lookup(name) else { panic!("not a forwarded integer: {name}") };
        self.ints[i as usize] = value;
    }
    pub(crate) fn names() -> impl Iterator<Item = &'static str> {
        C_VARIABLES.iter().map(|v| v.name)
    }
    pub(crate) fn lookup(&self, name: &str) -> Option<Fwd> {
        self.by_name.get(name).copied()
    }
    /// Every collector-visible Value held outside value cells and buffers.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.fundamental_mode_name);
        visit(&mut self.unbound_symbol);
        for v in self.objects.iter_mut().chain(self.defaults.iter_mut()) {
            visit(v);
        }
    }
    #[cfg(test)]
    pub(crate) fn object(&self, fwd: Fwd) -> Value {
        match fwd {
            Fwd::Obj(i) => self.objects[i as usize],
            Fwd::Buffer(i) => self.defaults[i as usize],
            _ => Value::NIL,
        }
    }
}

/// A buffer's per-buffer variable values (GNU keeps them in `struct buffer`
/// and a `local_flags` bit each). `None`, or an index past the end, is "not
/// local here": the buffer sees the default.
#[derive(Clone, Default)]
pub(crate) struct PerBuffer(Vec<Option<Value>>, pub(crate) VisitedModtime, Option<(u64, u64)>, Option<String>, Option<Value>);

/// Native scalar state, not Lisp Values: safe through moving collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum VisitedModtime {
    #[default]
    Unknown,
    Missing,
    Known { seconds: i64, nanos: i64, size: Option<u64> },
}

pub(crate) fn file_identity(path: &str) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

impl PerBuffer {
    pub(crate) fn editor_mark(&self) -> Option<Value> { self.4 }
    pub(crate) fn set_editor_mark(&mut self, marker: Value) { self.4 = Some(marker); }
    pub(crate) fn get(&self, i: u16) -> Option<Value> {
        self.0.get(i as usize).copied().flatten()
    }
    fn set(&mut self, i: u16, value: Value) {
        let i = i as usize;
        if self.0.len() <= i {
            self.0.resize(i + 1, None);
        }
        self.0[i] = Some(value);
    }
    /// Forget one binding without disturbing the other values or visit metadata.
    /// Absence is the local flag: removing the value also removes that flag.
    pub(crate) fn clear_local(&mut self, i: u16) {
        if let Some(value) = self.0.get_mut(i as usize) {
            *value = None;
        }
    }
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        if let Some(marker) = &mut self.4 { visit(marker); }
        for v in self.0.iter_mut().flatten() {
            visit(v);
        }
    }
}

/// GNU `enum Set_Internal_Bind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetFlag {
    Set,
    Bind,
}

/// The text a forwarded store fault takes across VM2's string-error
/// boundaries; [`Vm::forwarded_condition`] turns it back into GNU's condition.
pub(crate) fn fault_text(fault: Fault) -> String {
    match fault {
        Fault::Wrong(predicate, _) => format!("{WRONG_TYPE}{predicate}"),
        Fault::Error(message) => message,
        _ => "invalid forwarded variable fault".into(),
    }
}
const WRONG_TYPE: &str = "forwarded variable wrong-type-argument ";
const UNBINDABLE: &str = "Built-in variable may not be unbound : ";

impl Vm {
    /// GNU `SYMBOL_FORWARDED`: the forwarding of interned SLOT, if it names a
    /// built-in variable. Initialises and caches SLOT's lazy redirect.
    pub(super) fn forwarding(&mut self, slot: usize) -> Option<Fwd> {
        if self.variable_redirects.get(slot).copied().flatten().is_none() {
            let name = self.symbol_names.get(slot)?.clone();
            if self.names.get(&name) != Some(&slot) {
                return None;
            }
            self.initialize_variable_redirect(&name, slot);
        }
        match self.variable_redirects.get(slot).copied().flatten() {
            Some(VariableRedirect::Forwarded(fwd)) => Some(fwd),
            _ => None,
        }
    }
    /// [`Vm::forwarding`] without caching, for `&self` readers.
    pub(super) fn forwarding_ref(&self, slot: usize) -> Option<Fwd> {
        match self.variable_redirects.get(slot).copied().flatten() {
            Some(VariableRedirect::Forwarded(fwd)) => Some(fwd),
            Some(_) => None,
            None => {
                let name = self.symbol_names.get(slot)?;
                (self.names.get(name) == Some(&slot)).then(|| self.forwarded.lookup(name)).flatten()
            }
        }
    }
    pub(super) fn localized_boolean(&self, slot: usize) -> Option<Fwd> {
        match self.variable_redirects.get(slot).copied().flatten() {
            Some(VariableRedirect::LocalizedForwarded(fwd @ Fwd::Bool(_))) => Some(fwd),
            _ => None,
        }
    }
    pub(super) fn native_forwarding(&mut self, slot: usize) -> Option<Fwd> {
        self.localized_boolean(slot).or_else(|| self.forwarding(slot))
    }
    pub(super) fn check_boolean_default(&self, slot: usize, value: Value) -> Result<()> {
        if self.localized_boolean(slot).is_some() && value != Value::NIL && value != Value::TRUE {
            return Err(format!("VM2 localized Boolean default requires nil or t: {}", self.symbol_names[slot]));
        }
        Ok(())
    }
    /// Install a C_SPECIALS variable when its symbol is first interned.
    pub(super) fn initialize_c_special(&mut self, name: &str, slot: usize) {
        static INDEX: std::sync::OnceLock<HashMap<&'static str, usize>> = std::sync::OnceLock::new();
        let index = INDEX.get_or_init(|| C_SPECIALS.iter().enumerate().map(|(i, v)| (v.name, i)).collect());
        let Some(&i) = index.get(name) else { return; };
        let value = match C_SPECIALS[i].kind {
            Kind::Lisp(init) => self.materialize(init),
            Kind::Bool(b) => if b { Value::TRUE } else { Value::NIL },
            Kind::Int(n) => Value::integer(n).expect("C initial value is a fixnum"),
            Kind::PerBuffer { .. } => unreachable!("C_SPECIALS holds no per-buffer variables"),
        };
        self.slots[slot].value = value;
        if !NOT_SPECIAL.contains(&name) {
            self.specials.insert(name.to_owned());
        }
        // GNU's syms_of_* call Fmake_variable_buffer_local on these.
        if AUTOMATICALLY_LOCAL.contains(&name) {
            self.slots[slot].explicit_local = false;
            let table = self.allocate_object(data::Object::HashTable(Box::new(hash::HashTable::new_local_bindings())));
            self.install_local_table(slot, table, false);
        }
    }
    fn materialize(&mut self, init: Init) -> Value {
        match init {
            Init::Nil => Value::NIL,
            Init::True => Value::TRUE,
            Init::Fixnum(n) => Value::integer(n).expect("C initial value is a fixnum"),
            Init::Symbol(name) => self.intern(name),
            // build_string: unibyte unless the text has a non-ASCII char.
            Init::Str(text) => self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii()),
            Init::Float(x) => self.allocate_object(data::Object::Float(x)),
            Init::Getenv(name, fallback) => {
                let text = std::env::var(name).unwrap_or_else(|_| fallback.to_owned());
                self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii())
            }
            Init::SystemName => prim::prims::sysname::initial_value(self),
            Init::SourceDirectory => {
                // CARGO_MANIFEST_DIR is captured while building this crate.
                // Like GNU's expansion, this is lexical: do not canonicalize
                // or require that the build tree still exists at runtime.
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent().unwrap().parent().unwrap();
                let text = format!("{}/", root.to_str().expect("build source path is UTF-8"));
                self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii())
            }
            Init::List(items) => {
                let values: Vec<Value> = items.iter().map(|&i| self.materialize(i)).collect();
                self.list_value(&values)
            }
            Init::EtcDirectory => {
                let root = std::env::var_os("ROMACS_RUNTIME_ROOT").map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
                let text = format!("{}/", root.join("etc").display());
                self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii())
            }
            Init::Invocation(name) => {
                let exe = std::env::current_exe().unwrap_or_default();
                let text = if name {
                    exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    exe.parent().map(|d| format!("{}/", d.display())).unwrap_or_default()
                };
                self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii())
            }
            Init::UserIdentity(name) => prim::prims::user_identity::initial_value(self, name),
            Init::NilVector(n) => self.allocate_object(data::Object::Vector(vec![Value::NIL; n])),
            Init::Cons(car, cdr) => {
                let car = self.materialize(*car);
                let cdr = self.materialize(*cdr);
                self.keymap_cons(car, cdr)
            }
            Init::CharTable(spec) => {
                let purpose = spec.purpose.map_or(Value::NIL, |name| self.intern(name));
                let init = self.materialize(spec.init);
                let mut table = crate::char_table::CharTable::new(purpose);
                table.initialize(spec.extras.len(), init).expect("small char-table");
                for (slot, &extra) in spec.extras.iter().enumerate() {
                    table.extras[slot] = self.materialize(extra);
                }
                for &(from, to, value) in spec.ranges {
                    let value = self.materialize(value);
                    table.set_range(from as usize, to as usize, value).expect("C initial range");
                }
                self.allocate_object(data::Object::CharTable(Box::new(table)))
            }
        }
    }
    fn forwarded_object(&mut self, i: u16) -> Value {
        let value = self.forwarded.objects[i as usize];
        if value != Value::UNBOUND {
            return value;
        }
        let Kind::Lisp(init) = C_VARIABLES[self.forwarded.object_vars[i as usize]].kind else { unreachable!() };
        let value = self.materialize(init);
        self.forwarded.objects[i as usize] = value;
        value
    }
    /// GNU's private Qunbound symbol, exposed by the pinned permanent-hook fault.
    pub(super) fn forwarded_unbound_symbol(&mut self) -> Value {
        if self.forwarded.unbound_symbol == Value::UNBOUND {
            let symbol = self.generated_symbol(expand::generated_id());
            self.symbol_names[symbol.index(7).unwrap() - 1] = "unbound".into();
            self.forwarded.unbound_symbol = symbol;
        }
        self.forwarded.unbound_symbol
    }
    fn per_buffer_initial(&mut self, i: u16) -> Value {
        let variable = &C_VARIABLES[self.forwarded.buffer_vars[i as usize]];
        if variable.name == "mode-name" {
            if self.forwarded.fundamental_mode_name == Value::UNBOUND {
                self.forwarded.fundamental_mode_name = self.materialize(Init::Str("Fundamental"));
            }
            self.forwarded.fundamental_mode_name
        } else {
            let Kind::PerBuffer { default, .. } = variable.kind else { unreachable!() };
            self.materialize(default)
        }
    }
    fn per_buffer_default(&mut self, i: u16) -> Value {
        let value = self.forwarded.defaults[i as usize];
        if value != Value::UNBOUND {
            return value;
        }
        let Kind::PerBuffer { default, .. } = C_VARIABLES[self.forwarded.buffer_vars[i as usize]].kind else { unreachable!() };
        let value = self.materialize(default);
        self.forwarded.defaults[i as usize] = value;
        value
    }
    fn per_buffer(&self, buffer: usize) -> Option<&PerBuffer> {
        match self.objects.get(buffer)? {
            data::Object::Buffer(b) => Some(&b.per_buffer),
            data::Object::ByteBuffer(b) => Some(&b.per_buffer),
            _ => None,
        }
    }
    fn per_buffer_mut(&mut self, buffer: usize) -> Option<&mut PerBuffer> {
        match self.objects.get_mut(buffer)? {
            data::Object::Buffer(b) => Some(&mut b.per_buffer),
            data::Object::ByteBuffer(b) => Some(&mut b.per_buffer),
            _ => None,
        }
    }
    /// GNU `PER_BUFFER_VALUE_P (buf, idx)`.
    pub(super) fn per_buffer_always_local(&self, i: u16) -> bool {
        matches!(C_VARIABLES[self.forwarded.buffer_vars[i as usize]].name,
            "buffer-invisibility-spec" | "default-directory" | "buffer-file-name" | "buffer-file-truename" | "buffer-read-only" | "buffer-file-format" | "local-minor-modes" | "major-mode" | "mode-name" | "mark-active" | "buffer-undo-list" | "buffer-auto-save-file-name")
    }
    pub(super) fn clear_forwarded_local(&mut self, buffer: usize, i: u16) {
        if !self.per_buffer_always_local(i) {
            self.per_buffer_mut(buffer).expect("buffer").clear_local(i);
        }
    }
    pub(super) fn forwarded_buffer_value(&mut self, buffer: usize, i: u16) -> Value {
        if let Some(value) = self.per_buffer(buffer).and_then(|p| p.get(i)) { return value; }
        if self.per_buffer_always_local(i) {
            let value = self.per_buffer_initial(i);
            self.per_buffer_mut(buffer).expect("buffer").set(i, value);
            value
        } else { self.per_buffer_default(i) }
    }
    pub(super) fn reset_forwarded_modes(&mut self, buffer: usize) {
        for name in ["major-mode", "mode-name"] {
            let Some(Fwd::Buffer(i)) = self.forwarded.lookup(name) else { unreachable!() };
            self.per_buffer_mut(buffer).expect("buffer").clear_local(i);
            self.forwarded_buffer_value(buffer, i);
        }
    }
    pub(super) fn reset_forwarded_visit(&mut self, buffer: usize) {
        for name in ["buffer-file-name", "buffer-file-truename", "buffer-read-only", "buffer-file-format"] {
            let Some(Fwd::Buffer(i)) = self.forwarded.lookup(name) else { unreachable!() };
            self.per_buffer_mut(buffer).expect("buffer").clear_local(i);
        }
        let locals = self.per_buffer_mut(buffer).expect("buffer");
        locals.1 = VisitedModtime::Unknown;
        locals.2 = None;
        locals.3 = None;
    }
    /// BVAR (BUFFER, undo_list), and bset_undo_list.
    pub(crate) fn undo_list_of(&mut self, buffer: usize) -> Value {
        let i = self.undo_list_index();
        self.forwarded_buffer_value(buffer, i)
    }
    pub(crate) fn set_undo_list_of(&mut self, buffer: usize, value: Value) {
        let i = self.undo_list_index();
        if let Some(locals) = self.per_buffer_mut(buffer) { locals.set(i, value); }
    }
    fn undo_list_index(&mut self) -> u16 {
        if self.undo_list_slot == u16::MAX {
            let Some(Fwd::Buffer(i)) = self.forwarded.lookup("buffer-undo-list") else { unreachable!("declared") };
            self.undo_list_slot = i;
        }
        self.undo_list_slot
    }
    /// The directory a new buffer starts with: the current buffer's
    /// (reset_buffer), or before any buffer exists the process directory as
    /// a directory name (init_buffer). init_buffer makes it unibyte and GNU's
    /// startup.el later decodes it; VM2 runs no such startup, so it is
    /// decoded here: unibyte when ASCII (as GNU's decoding leaves it),
    /// multibyte when other UTF-8, else its raw bytes. No file
    /// name handler exists yet when the first buffer is made, so
    /// init_buffer's `/:' quoting never applies.
    pub(crate) fn inherited_directory(&mut self) -> Value {
        let Some(Fwd::Buffer(i)) = self.forwarded.lookup("default-directory") else { unreachable!("declared") };
        let current = self.slots[self.buffer_slot].value;
        if let Some(buffer) = current.index_if(0).and_then(|n| n.checked_sub(1))
            && self.per_buffer(buffer).is_some()
        {
            return self.forwarded_buffer_value(buffer, i);
        }
        // init_buffer starts from emacs_wd, a symlinked PWD's spelling kept.
        match crate::environment::startup_directory() {
            Some(dir) => {
                let mut bytes = dir.to_vec();
                if bytes.last() != Some(&b'/') { bytes.push(b'/'); }
                let decoded = !bytes.is_ascii() && std::str::from_utf8(&bytes).is_ok();
                self.make_lisp_string(bytes, decoded)
            }
            None => Value::NIL,
        }
    }
    /// bset_directory (BUFFER, DIRECTORY).
    pub(crate) fn set_directory_of(&mut self, buffer: usize, directory: Value) {
        let Some(Fwd::Buffer(i)) = self.forwarded.lookup("default-directory") else { unreachable!("declared") };
        if let Some(locals) = self.per_buffer_mut(buffer) { locals.set(i, directory); }
    }
    pub(super) fn per_buffer_local_p(&self, buffer: usize, i: u16) -> bool {
        self.per_buffer_always_local(i) || self.per_buffer(buffer).and_then(|p| p.get(i)).is_some()
    }
    fn current_buffer_object(&mut self) -> std::result::Result<usize, Fault> {
        self.buffer_index().map_err(Fault::Error)
    }
    /// Snapshot live visits without changing current-buffer or dynamic bindings.
    /// Callers are leaf primitives: no collection occurs while Values are held.
    pub(crate) fn file_visits(&self) -> Vec<(Value, String, Option<(u64, u64)>, Option<String>)> {
        let Some(Fwd::Buffer(i)) = self.forwarded.lookup("buffer-file-name") else { return Vec::new() };
        // In the buffer list's order, as get-file-buffer walks Vbuffer_alist.
        let mut buffers = self.buffer_order_snapshot();
        let current = self.slots[self.buffer_slot].value;
        if !buffers.contains(&current) { buffers.push(current); }
        buffers.into_iter().filter_map(|buffer| {
            let index = buffer.index(0).ok()?.checked_sub(1)?;
            let locals = self.per_buffer(index)?;
            let name = self.eval_buffer_name(locals.get(i)?).ok()??;
            Some((buffer, name, locals.2, locals.3.clone()))
        }).collect()
    }

    /// GNU `bset_<field> (current_buffer, VALUE)`: write the current buffer's
    /// own slot of the per-buffer variable NAME and flag it local, bypassing
    /// `let` bindings and the predicate, as C does when it sets a BVAR
    /// directly (fileio.c:5157 `bset_filename`).
    pub(crate) fn bset_current(&mut self, name: &str, value: Value) -> std::result::Result<(), Fault> {
        let Some(Fwd::Buffer(i)) = self.forwarded.lookup(name) else {
            return Err(Fault::Error(format!("{name} is not a VM2 per-buffer variable")));
        };
        let buffer = self.current_buffer_object()?;
        if name == "buffer-file-name" {
            let identity = self.eval_buffer_name(value).ok().flatten()
                .and_then(|path| file_identity(&path));
            let truename = self.eval_buffer_name(value).ok().flatten().map(|path| {
                std::fs::canonicalize(&path).ok().and_then(|p| p.to_str().map(str::to_owned)).unwrap_or(path)
            });
            let locals = self.per_buffer_mut(buffer).expect("live buffer");
            locals.2 = identity;
            locals.3 = truename;
        }
        self.per_buffer_mut(buffer).expect("live buffer").set(i, value);
        Ok(())
    }
    /// Native storage, including the default of a localized Boolean.
    /// Effective reads go through `variable_value` and its local table.
    pub(super) fn forwarded_value(&mut self, fwd: Fwd) -> Result<Value> {
        Ok(match fwd {
            Fwd::Obj(i) => self.forwarded_object(i),
            Fwd::Bool(i) => if self.forwarded.bools[i as usize] { Value::TRUE } else { Value::NIL },
            Fwd::Int(i) => Value::integer(self.forwarded.ints[i as usize])?,
            Fwd::Buffer(i) => {
                let buffer = self.buffer_index()?;
                self.forwarded_buffer_value(buffer, i)
            }
        })
    }
    /// GNU `default_value`, SYMBOL_FORWARDED arm: a per-buffer variable's
    /// default, otherwise the current value.
    pub(super) fn forwarded_default(&mut self, fwd: Fwd) -> Result<Value> {
        match fwd {
            Fwd::Buffer(i) => Ok(self.per_buffer_default(i)),
            _ => self.forwarded_value(fwd),
        }
    }
    /// GNU `store_symval_forwarding` into BUFFER (an object index; ignored
    /// unless FWD is per-buffer).
    fn store_forwarded(&mut self, fwd: Fwd, value: Value, buffer: usize) -> std::result::Result<(), Fault> {
        match fwd {
            Fwd::Obj(i) => self.forwarded.objects[i as usize] = value,
            Fwd::Bool(i) => self.forwarded.bools[i as usize] = value != Value::NIL,
            Fwd::Int(i) => {
                // CHECK_INTEGER; VM2 has no bignums, so every integer fits
                // and GNU's overflow-error arm is unreachable.
                let n = value.integer_value().ok_or(Fault::Wrong("integerp", value))?;
                self.forwarded.ints[i as usize] = n;
            }
            Fwd::Buffer(i) => {
                if value != Value::NIL {
                    let Kind::PerBuffer { predicate, .. } = C_VARIABLES[self.forwarded.buffer_vars[i as usize]].kind else { unreachable!() };
                    match predicate {
                        Some(Predicate::Integerp) if value.integer_value().is_none() => {
                            return Err(Fault::Wrong("integerp", value));
                        }
                        Some(Predicate::Symbolp) if value != Value::TRUE && value.index_if(7).is_none() => {
                            return Err(Fault::Wrong("symbolp", value));
                        }
                        Some(Predicate::Stringp) if self.str_view(value).is_none() => {
                            return Err(Fault::Wrong("stringp", value));
                        }
                        _ => {}
                    }
                }
                // GNU writes the buffer's slot whether or not its flag is
                // set; an unflagged slot always mirrors the default, so VM2
                // keeps only flagged (local) values.
                if self.per_buffer_local_p(buffer, i) {
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, value);
                }
            }
        }
        Ok(())
    }
    /// GNU `let_shadows_buffer_binding_p`: a `let` of SLOT's default made
    /// while BUFFER was current is in effect.
    fn let_shadows_buffer_binding(&self, slot: usize, buffer: Value) -> bool {
        self.dynamic.iter().any(|&(bound, _, owner, let_default)| bound == slot && let_default && owner == buffer)
    }
    /// GNU `set_internal` (data.c:1806): setting per-buffer variable I with
    /// no local value in BUFFER, within a `let' of its default made there,
    /// sets the default through set_default_internal.
    pub(super) fn sets_shadowed_default(&self, slot: usize, i: u16, buffer: usize) -> bool {
        !self.per_buffer_local_p(buffer, i) && self.let_shadows_buffer_binding(slot, self.slots[self.buffer_slot].value)
    }
    /// GNU `set_internal`, SYMBOL_FORWARDED arm (data.c:1791-1818).
    pub(super) fn set_forwarded(&mut self, slot: usize, fwd: Fwd, value: Value, flag: SetFlag) -> std::result::Result<(), Fault> {
        if value == Value::UNBOUND {
            return Err(Fault::Error(format!("{UNBINDABLE}{}", self.symbol_names[slot])));
        }
        let mut buffer = 0;
        if let Fwd::Buffer(i) = fwd {
            buffer = self.current_buffer_object()?;
            if flag == SetFlag::Set && !self.per_buffer_local_p(buffer, i) {
                if self.sets_shadowed_default(slot, i, buffer) {
                    self.set_forwarded_default(slot, fwd, value, flag)?;
                } else {
                    // SET_PER_BUFFER_VALUE_P before the predicate check, as
                    // GNU: a rejected value still leaves the buffer local.
                    let current = self.per_buffer_default(i);
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, current);
                }
            }
        }
        self.store_forwarded(fwd, value, buffer)
    }
    /// GNU `set_default_internal`, SYMBOL_FORWARDED arm (data.c:2085-2124):
    /// a per-buffer default is stored WITHOUT the predicate.
    pub(super) fn set_forwarded_default(&mut self, slot: usize, fwd: Fwd, value: Value, flag: SetFlag) -> std::result::Result<(), Fault> {
        self.check_boolean_default(slot, value).map_err(Fault::Error)?;
        match fwd {
            Fwd::Buffer(i) => {
                self.forwarded.defaults[i as usize] = value;
                Ok(())
            }
            _ => self.set_forwarded(slot, fwd, value, flag),
        }
    }
    /// GNU `specbind` for a forwarded SLOT (the end of any alias chain): the
    /// binding it records, (SLOT OLD OWNER LET-DEFAULT). A per-buffer
    /// variable with no local value here, or a localized Boolean, binds its
    /// default (SPECPDL_LET_DEFAULT).
    pub(super) fn forwarded_binding(&mut self, slot: usize, fwd: Fwd) -> std::result::Result<(usize, Value, Value, bool), Fault> {
        let old = self.forwarded_value(fwd).map_err(Fault::Error)?;
        let owner = self.slots[self.buffer_slot].value;
        // Only a per-buffer variable needs the current buffer; binding any
        // other forwarded variable must not create the initial one.
        let let_default = self.localized_boolean(slot).is_some() || match fwd {
            Fwd::Buffer(i) => {
                let buffer = self.current_buffer_object()?;
                !self.per_buffer_local_p(buffer, i)
            }
            _ => false,
        };
        Ok((slot, old, owner, let_default))
    }
    /// GNU `do_specbind`'s store for a forwarded SLOT, once its binding is
    /// recorded: a failed store leaves that binding to restore OLD.
    pub(super) fn store_forwarded_binding(&mut self, slot: usize, fwd: Fwd, value: Value, let_default: bool) -> std::result::Result<(), Fault> {
        let value = if self.localized_boolean(slot).is_none() { value } else if value == Value::NIL { Value::NIL } else { Value::TRUE };
        if let_default {
            self.set_forwarded_default(slot, fwd, value, SetFlag::Bind)
        } else {
            self.set_forwarded(slot, fwd, value, SetFlag::Bind)
        }
    }
    /// GNU `do_one_unbind` for a forwarded binding of SLOT. A default
    /// binding, and a global variable's SPECPDL_LET, restore through
    /// set_default_internal, which reports `set'; for a variable still
    /// global it then calls set_internal, which reports `unlet'.
    /// SPECPDL_LET_LOCAL reports `unlet' in OWNER and restores, only if
    /// OWNER still has its local value. A watcher's signal leaves the value
    /// unrestored, as does a saved integer that set-default-toplevel-value
    /// replaced with a non-integer, which signals once the watchers ran.
    pub(super) fn unbind_forwarded(&mut self, slot: usize, fwd: Fwd, old: Value, owner: Value, let_default: bool) -> Result<()> {
        let local = owner.index_if(0).and_then(|n| n.checked_sub(1));
        if let Fwd::Buffer(i) = fwd && !let_default && !local.is_some_and(|buffer| self.per_buffer_local_p(buffer, i)) {
            return Ok(());
        }
        if self.variable_trapped(slot) {
            if matches!(fwd, Fwd::Buffer(_)) && !let_default {
                self.notify_watchers_at(slot, old, "unlet", Some(owner))?;
            } else {
                self.notify_watchers_at(slot, old, "set", Some(Value::NIL))?;
                if !matches!(fwd, Fwd::Buffer(_)) && self.localized_boolean(slot).is_none() {
                    self.notify_watchers(slot, old, "unlet")?;
                }
            }
        }
        match fwd {
            Fwd::Obj(i) => self.forwarded.objects[i as usize] = old,
            Fwd::Bool(i) => self.forwarded.bools[i as usize] = old != Value::NIL,
            Fwd::Int(i) => {
                let Some(n) = old.integer_value() else {
                    let predicate = self.intern("integerp");
                    let condition = self.intern("wrong-type-argument");
                    let data = self.list_value(&[predicate, old]);
                    self.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
                    return Err("forwarded variable wrong-type-argument integerp".into());
                };
                self.forwarded.ints[i as usize] = n;
            }
            Fwd::Buffer(i) if let_default => self.forwarded.defaults[i as usize] = old,
            Fwd::Buffer(i) => self.per_buffer_mut(local.expect("live owner")).expect("live buffer").set(i, old),
        }
        Ok(())
    }
    /// GNU `make-local-variable` on a forwarded variable (data.c:2245).
    pub(super) fn make_forwarded_local(&mut self, slot: usize, fwd: Fwd) -> Result<()> {
        match fwd {
            Fwd::Bool(_) => {
                self.promote_boolean_local(slot, true)?;
                self.make_local_variable(slot)
            }
            Fwd::Buffer(i) => {
                let buffer = self.buffer_index()?;
                if !self.per_buffer_local_p(buffer, i) {
                    let current = self.per_buffer_default(i);
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, current);
                }
                Ok(())
            }
            _ => Err(unlocalizable(&self.symbol_names[slot])),
        }
    }
    /// The condition GNU signals for a forwarded store fault rendered by
    /// [`fault_text`], with VALUE the value being stored.
    pub(super) fn forwarded_condition(&mut self, error: &str, value: Value) -> Option<(Value, Value)> {
        let (condition, data) = if let Some(predicate) = error.strip_prefix(WRONG_TYPE) {
            let predicate = self.intern(predicate);
            ("wrong-type-argument", vec![predicate, value])
        } else if error.starts_with(UNBINDABLE) || error.starts_with(UNLOCALIZABLE) {
            let message = self.make_lisp_string(error.as_bytes().to_vec(), true);
            ("error", vec![message])
        } else {
            return None;
        };
        let condition = self.intern(condition);
        Some((condition, self.list_value(&data)))
    }
}

/// The DEFVAR_LISP variables `insert-file-contents' and `write-region' read
/// (all nil in C; GNU's -Q image fills `file-name-handler-alist',
/// `file-coding-system-alist' and `set-auto-coding-function' from Lisp, VM2
/// keeps nil). Declared special with GNU's C value so a `let' of one is
/// dynamic and seen by the primitive, but NOT forwarded: forwarded object
/// variables cannot be made buffer-local yet (VM2-CVAR1), and packages make
/// the change hooks and `deactivate-mark' local.
const FILEIO_SPECIALS: [(&str, &str); 18] = [
    ("first-change-hook", "buffer.c:5738"),
    ("inhibit-read-only", "buffer.c:5897"),
    ("before-change-functions", "buffer.c:5704"),
    ("after-change-functions", "buffer.c:5720"),
    ("coding-system-for-read", "coding.c:11971"),
    ("coding-system-for-write", "coding.c:11980"),
    ("last-coding-system-used", "coding.c:11994"),
    ("file-coding-system-alist", "coding.c:12030"),
    ("select-safe-coding-system-function", "coding.c:12152"),
    ("file-name-handler-alist", "fileio.c:6854"),
    ("inhibit-file-name-handlers", "fileio.c:6943"),
    ("inhibit-file-name-operation", "fileio.c:6948"),
    ("set-auto-coding-function", "fileio.c:6876"),
    ("after-insert-file-functions", "fileio.c:6889"),
    ("write-region-annotate-functions", "fileio.c:6899"),
    ("write-region-post-annotation-function", "fileio.c:6925"),
    ("deactivate-mark", "keyboard.c:14030"),
    // Initially nil; cus-start.el's delayed initializer chooses the default.
    ("temporary-file-directory", "filelock.c:814"),
];

impl Vm {
    pub(crate) fn declare_fileio_specials(&mut self) {
        self.declare_special_raw("change-major-mode-hook", Value::NIL);
        self.declare_special_raw("create-lockfiles", Value::TRUE);
        // GNU coding.c:12084-12088, before Lisp selects a locale coding.
        self.declare_special_raw("locale-coding-system", Value::NIL);
        // cmds.c and undo.c, read by self-insert-command and record_point.
        self.declare_special_raw("post-self-insert-hook", Value::NIL);
        self.declare_special_raw("undo-inhibit-record-point", Value::NIL);
        for (name, _gnu) in FILEIO_SPECIALS {
            if !self.specials.contains(name) {
                self.declare_special_raw(name, Value::NIL);
            }
        }
    }
}

const UNLOCALIZABLE: &str = "VM2 cannot make built-in variable buffer-local: ";
/// GNU turns an object, boolean or integer built-in variable into
/// SYMBOL_LOCALIZED with a forwarded realvalue (`make_blv`). VM2 does not
/// model that combination yet and refuses instead (DIVERGENCES.md
/// VM2-CVAR1), rather than silently creating a binding nothing would read.
pub(super) fn unlocalizable(name: &str) -> String {
    format!("{UNLOCALIZABLE}{name}")
}

#[cfg(test)]
#[path = "forwarded_tests.rs"]
mod forwarded_tests;
