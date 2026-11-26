//! An owner's logical windows (ownership O1b §2.1, §4.1): the primitives
//! lisp/vm/owner-window.el lays its master's windows out with, over
//! `crate::owner::display`. Not in GNU.
use crate::Value;

// (romacs--owner-display-start SLOT): lay this owner's windows out as
// owner slot SLOT from here, with the display its host attached: its
// fonts are the slot's, and its layout answers the windows' queries.
// t; nil, its argument unread and nothing changed, when no display is
// attached.
defsubr! { name = "romacs--owner-display-start", gnu = "romacs:owner", effect = Global,
fn owner_display_start(cx, slot: Value) {
    cx.owner_display_start(slot)
}}

// (romacs--owner-set-cell LINE COLUMN): make the canonical character
// cell LINE by COLUMN px, which every window's lines and columns count
// in; the owner's own frame keeps its size where two cells fit each way.
// Nil; an error for a cell two of which do not fit a frame.
defsubr! { name = "romacs--owner-set-cell", gnu = "romacs:owner", effect = Global,
fn owner_set_cell(cx, line: Value, column: Value) {
    cx.owner_set_cell(line, column)
}}

// (romacs--owner-panes-prepare): run what a redisplay runs before it
// lays windows out, `pre-redisplay-function' and then the window change
// functions, and forget the layouts of windows no longer live.  Nil.
defsubr! { name = "romacs--owner-panes-prepare", gnu = "romacs:owner", effect = Global,
fn owner_panes_prepare(cx) {
    cx.owner_panes_prepare()
}}

// (romacs--owner-pane-lay-out WINDOW MASTER VIEW REVISION CATALOG SELECTED):
// lay live WINDOW out as a frame of its own box, for request REVISION of
// master window MASTER under the master's VIEW and font CATALOG, its
// cursor active when SELECTED, and keep it as the window's last picture;
// its ends answer the window's queries.  Nil.
defsubr! { name = "romacs--owner-pane-lay-out", gnu = "romacs:owner", effect = Global,
fn owner_pane_lay_out(cx, window: Value, master: Value, view: Value, revision: Value, catalog: Value, selected: Value) {
    cx.owner_pane_lay_out(window, [master, view, revision, catalog, selected])
}}

// (romacs--owner-pane-picture WINDOW): live WINDOW's last picture, a
// unibyte string (a format word, then Postcard), under the next visual
// seq; nil before any layout.
defsubr! { name = "romacs--owner-pane-picture", gnu = "romacs:owner", effect = Global,
fn owner_pane_picture(cx, window: Value) {
    cx.owner_pane_picture(window)
}}
