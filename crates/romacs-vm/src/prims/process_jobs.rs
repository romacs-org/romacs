use super::*;

 // Start the command ARGV, a list of strings, in the background in DIRECTORY, with the
 // string INPUT on its standard input, killed after SECONDS.  Returns its id, a number that
 // `romacs--process-result' and `romacs--process-forget' take.
 defsubr! { name = "romacs--process-start", gnu = "romacs:process", effect = Global,
fn process_start(cx, argv: Value, input: Value, directory: Value, seconds: Integer) {
    cx.process_start(argv, input, directory, seconds.0)
}}

// The result of background command ID once it has finished: (STATUS STDOUT STDERR
// ERROR), STATUS its exit code or nil, STDOUT and STDERR its output as strings, ERROR a
// string when it could not run, timed out or was cancelled, else nil.  nil while it still
// runs.
defsubr! { name = "romacs--process-result", gnu = "romacs:process", effect = Global,
fn process_result(cx, id: Integer) { cx.process_result(id.0) }}

// The ids of the background commands that have finished and not yet been taken, in
// the order they started.
defsubr! { name = "romacs--process-ready", gnu = "romacs:process", effect = Global,
fn process_ready(cx) { cx.process_ready() }}

// Whether a background command has finished, without taking its result.
defsubr! { name = "romacs--process-pending-p", gnu = "romacs:process", effect = Reads,
fn process_pending_p(cx) { cx.process_pending_p() }}

// Forget background command ID: a command still running is cancelled, and its result
// is dropped.  Returns nil.
defsubr! { name = "romacs--process-forget", gnu = "romacs:process", effect = Global,
fn process_forget(cx, id: Integer) { cx.process_forget(id.0) }}

// Parse the JSON text SOURCE, a string: an object as an alist with string keys, an
// array as a list, null as `json-null', false as `json-false', true as t, and a number as
// an integer or a float.  Signals an error on invalid JSON.
defsubr! { name = "romacs-json-read", gnu = "romacs:json", effect = Pure,
fn process_json_read(cx, source: Value) { cx.process_json_read(source) }}
