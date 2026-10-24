use super::*;

defsubr! { name = "romacs--term-create", gnu = "romacs:terminal", effect = Global,
fn terminal_create(cx, shell: Value, directory: Value, columns: Integer, rows: Integer) {
    cx.terminal_create(shell, directory, columns.0, rows.0)
}}

defsubr! { name = "romacs--term-send", gnu = "romacs:terminal", effect = Global,
fn terminal_send(cx, session: Integer, bytes: Value) {
    cx.terminal_send(session.0, bytes)
}}

defsubr! { name = "romacs--term-follow", gnu = "romacs:terminal", effect = Global,
fn terminal_follow(cx, session: Integer, follow: Value) {
    cx.terminal_follow(session.0, follow)
}}

defsubr! { name = "romacs--term-close", gnu = "romacs:terminal", effect = Global,
fn terminal_close(cx, session: Integer) {
    cx.terminal_close(session.0)
}}

defsubr! { name = "romacs--term-running-p", gnu = "romacs:terminal", effect = Reads,
fn terminal_running(cx, session: Integer) {
    cx.terminal_running(session.0)
}}
