use super::*;

 // Start the command ARGV, a list of strings, in the background in DIRECTORY, with the
 // string INPUT on its standard input, killed after SECONDS.  Returns its id, a number that
 // `romacs--process-result' and `romacs--process-forget' take.  Signals an error with 16
 // commands kept already, an empty program, a relative DIRECTORY, an INPUT over 1 MiB, or
 // SECONDS outside 1 to 120.
 defsubr! { name = "romacs--process-start", gnu = "romacs:process", effect = Global,
fn process_start(cx, argv: Value, input: Value, directory: Value, seconds: Integer) {
    cx.process_start(argv, input, directory, seconds.0)
}}

// The result of background command ID once it has finished: (STATUS STDOUT STDERR
// ERROR), STATUS its exit code or nil, STDOUT and STDERR its output as strings, each cut at
// 4 MiB of bytes and decoded as UTF-8, invalid bytes replaced, ERROR a string when it could
// not run, timed out, was cancelled, wrote more than 4 MiB to either stream, or its pipes
// or the watch over it failed, else nil.  nil while it still runs.  The result stays until
// `romacs--process-forget'; an unknown ID signals an error.
defsubr! { name = "romacs--process-result", gnu = "romacs:process", effect = Global,
fn process_result(cx, id: Integer) { cx.process_result(id.0) }}

// The ids of the background commands still kept that have finished and that no
// earlier call reported, in the order they started: each id is reported at most once.
defsubr! { name = "romacs--process-ready", gnu = "romacs:process", effect = Global,
fn process_ready(cx) { cx.process_ready() }}

// Whether a background command may have finished since `romacs--process-ready' last
// ran: a hint for waking, which `romacs--process-ready' answers exactly.  Takes nothing.
defsubr! { name = "romacs--process-pending-p", gnu = "romacs:process", effect = Reads,
fn process_pending_p(cx) { cx.process_pending_p() }}

// Forget background command ID: a command still running is cancelled, and its result
// is dropped.  An unknown ID signals an error.  Returns nil.
defsubr! { name = "romacs--process-forget", gnu = "romacs:process", effect = Global,
fn process_forget(cx, id: Integer) { cx.process_forget(id.0) }}

// Parse the JSON text SOURCE, a string: an object as an alist with string keys, an
// array as a list, null as `json-null', false as `json-false', true as t, and a number as an
// integer when the parser reads it as one (no fraction or exponent, not -0, and within 64
// bits), else as a float.  Signals an error on invalid JSON, and on such an integer too
// large for a fixnum.
defsubr! { name = "romacs-json-read", gnu = "romacs:json", effect = Pure,
fn process_json_read(cx, source: Value) { cx.process_json_read(source) }}
