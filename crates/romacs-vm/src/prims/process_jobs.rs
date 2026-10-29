use super::*;

 defsubr! { name = "romacs--process-start", gnu = "romacs:process", effect = Global,
fn process_start(cx, argv: Value, input: Value, directory: Value, seconds: Integer) {
    cx.process_start(argv, input, directory, seconds.0)
}}

defsubr! { name = "romacs--process-result", gnu = "romacs:process", effect = Global,
fn process_result(cx, id: Integer) { cx.process_result(id.0) }}

defsubr! { name = "romacs--process-ready", gnu = "romacs:process", effect = Global,
fn process_ready(cx) { cx.process_ready() }}

defsubr! { name = "romacs--process-pending-p", gnu = "romacs:process", effect = Reads,
fn process_pending_p(cx) { cx.process_pending_p() }}

defsubr! { name = "romacs--process-forget", gnu = "romacs:process", effect = Global,
fn process_forget(cx, id: Integer) { cx.process_forget(id.0) }}

defsubr! { name = "romacs-json-read", gnu = "romacs:json", effect = Pure,
fn process_json_read(cx, source: Value) { cx.process_json_read(source) }}
