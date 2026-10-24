//! GNU char-table construction and extra slots.
use super::*;

defsubr! { name = "make-char-table", gnu = "chartab.c:104", effect = Reads,
fn make_char_table(cx, purpose: Value; optional init: Value) {
    cx.make_char_table(purpose, init)
}}

defsubr! { name = "char-table-extra-slot", gnu = "chartab.c:548", effect = Reads,
fn char_table_extra_slot(cx, table: Value, n: Value) {
    cx.char_table_extra(table, n, None)
}}

defsubr! { name = "set-char-table-extra-slot", gnu = "chartab.c:562", effect = MutatesArgs,
fn set_char_table_extra_slot(cx, table: Value, n: Value, value: Value) {
    cx.char_table_extra(table, n, Some(value))
}}

defsubr! { name = "char-table-parent", gnu = "chartab.c:510", effect = Reads,
fn char_table_parent(cx, table: Value) {
    cx.char_table_parent(table, None)
}}

defsubr! { name = "set-char-table-parent", gnu = "chartab.c:524", effect = MutatesArgs,
fn set_char_table_parent(cx, table: Value, parent: Value) {
    cx.char_table_parent(table, Some(parent))
}}

defsubr! { name = "set-char-table-range", gnu = "chartab.c:609", effect = MutatesArgs,
fn set_char_table_range(cx, table: Value, range: Value, value: Value) {
    cx.set_char_table_range(table, range, value)
}}

