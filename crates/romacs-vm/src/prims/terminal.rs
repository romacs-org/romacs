use super::*;

defsubr! { name = "romacs--term-create", gnu = "romacs:terminal", effect = Global,
fn terminal_create(cx, shell: Value, directory: Value; optional columns: Value, rows: Value) {
    cx.terminal_create(shell, directory, columns, rows)
}}

defsubr! { name = "romacs--term-send", gnu = "romacs:terminal", effect = Global,
fn terminal_send(cx, session: Integer, bytes: Value) {
    cx.terminal_send(session.0, bytes)
}}

defsubr! { name = "romacs--term-follow", gnu = "romacs:terminal", effect = Global,
fn terminal_follow(cx, session: Integer, follow: Value; optional window: Value) {
    cx.terminal_follow(session.0, follow, window)
}}

defsubr! { name = "romacs--term-track", gnu = "romacs:terminal", effect = Global,
fn terminal_track(cx) {
    cx.terminal_track()
}}

defsubr! { name = "romacs--term-paste", gnu = "romacs:terminal", effect = Global,
fn terminal_paste(cx, session: Integer, text: Value) {
    cx.terminal_paste(session.0, text)
}}

defsubr! { name = "romacs--term-close", gnu = "romacs:terminal", effect = Global,
fn terminal_close(cx, session: Integer) {
    cx.terminal_close(session.0)
}}

defsubr! { name = "romacs--term-running-p", gnu = "romacs:terminal", effect = Reads,
fn terminal_running(cx, session: Integer) {
    cx.terminal_running(session.0)
}}

defsubr! { name = "romacs--term-encode-event", gnu = "romacs:terminal", effect = Reads,
fn terminal_encode_event(cx, event: Value; optional application_cursor: Value) {
    cx.terminal_encode_event(event, application_cursor != Value::NIL)
}}

defsubr! { name = "romacs--term-send-event", gnu = "romacs:terminal", effect = Global,
fn terminal_send_event(cx, session: Integer, event: Value) {
    cx.terminal_send_event(session.0, event)
}}

defsubr! { name = "romacs--term-state", gnu = "romacs:terminal", effect = Reads,
fn terminal_state(cx, session: Integer) {
    cx.terminal_state(session.0)
}}
