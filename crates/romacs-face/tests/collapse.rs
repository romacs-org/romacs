//! The one-time collapse of `defface` condition specs, driven by the real
//! specs of the tree.

mod common;


use common::*;
use romacs_face::{Attribute, Background, Collapsed, Datum, FaceError, Slot};

fn fg(c: &Collapsed, mode: Background) -> Option<Datum> {
    c.get(mode)
        .map(|def| slot_datum(def.attrs().get(Attribute::Foreground)))
}

#[test]
fn tty_greyscale_and_8_colour_branches_are_discarded_and_background_kept() {
    let table = deffaces_table();
    let spec = &table
        .iter()
        .find(|(n, _)| n == "font-lock-comment-face")
        .unwrap()
        .1;
    let printed = spec.to_lisp();
    assert!(
        printed.contains("grayscale") && printed.contains("(min-colors 8)"),
        "{printed}"
    );
    let c = Collapsed::collapse(
        "font-lock-comment-face",
        &romacs_face::BoundedDatum::new(spec).unwrap(),
    )
    .unwrap();
    assert!(!c.is_uniform());
    assert_eq!(
        fg(&c, Background::Light),
        Some((*d("\"Firebrick\"")).clone())
    );
    assert_eq!(
        fg(&c, Background::Dark),
        Some((*d("\"chocolate1\"")).clone())
    );
    // The (t :weight bold :slant italic) fallback is never reached.
    assert_eq!(
        c.get(Background::Light)
            .unwrap()
            .attrs()
            .get(Attribute::Weight),
        &Slot::Unspecified
    );
}

#[test]
fn a_spec_for_terminals_only_matches_nothing() {
    let c = Collapsed::collapse(
        "s86",
        &d("((((type tty) (class color)) :foreground \"red\") (((class mono)) :inverse-video t))"),
    )
    .unwrap();
    assert!(c.get(Background::Light).is_none() && c.get(Background::Dark).is_none());
}

#[test]
fn default_entry_is_prepended_and_old_style_entries_read() {
    let c = Collapsed::collapse("s86", &d("((default :weight bold) (((background dark)) (:foreground \"white\")) (t :foreground \"black\"))")).unwrap();
    let dark = c.get(Background::Dark).unwrap();
    assert_eq!(slot_datum(dark.attrs().get(Attribute::Weight)), d("bold"));
    assert_eq!(fg(&c, Background::Dark), Some((*d("\"white\"")).clone()));
    assert_eq!(fg(&c, Background::Light), Some((*d("\"black\"")).clone()));
}

#[test]
fn unknown_display_requirement_is_an_error() {
    let err = Collapsed::collapse("s86", &d("((((frobs 3)) :foreground \"red\"))")).unwrap_err();
    assert!(
        matches!(
            err,
            FaceError::Spec {
                reason: "unknown display requirement",
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn specs_written_for_pgtk_match_romacs() {
    // Today's romacs reports window-system `romacs', and its faces.el only
    // maps `graphic' to it, so a (type x w32 ns haiku pgtk android) branch,
    // as cus-edit and eww write them, does not match and the face falls
    // through to its tty/fallback branch. The collapse treats romacs as the
    // pgtk display it replaces.
    let c = Collapsed::collapse(
        "s86",
        &d("((((type x w32 ns haiku pgtk android) (class color)) :box t) (t :inverse-video t))"),
    )
    .unwrap();
    let def = c.get(Background::Light).unwrap();
    assert_eq!(slot_datum(def.attrs().get(Attribute::Box)), d("1"));
    assert_eq!(def.attrs().get(Attribute::InverseVideo), &Slot::Unspecified);
    let table = deffaces_table();
    let pgtk_only = table
        .iter()
        .filter(|(_, s)| {
            let p = s.to_lisp();
            p.contains("(type ")
                && p.contains(" pgtk")
                && !p.contains("romacs")
                && !p.contains("graphic")
        })
        .count();
    eprintln!(
        "deffaces whose (type ...) branches name pgtk but neither romacs nor graphic: {pgtk_only}"
    );
    assert!(pgtk_only > 0);
}

#[test]
fn every_defface_spec_in_the_corpus_collapses() {
    let table = deffaces_table();
    let (mut two, mut conditional) = (0, 0);
    for (face, spec) in &table {
        let c = Collapsed::collapse(face, &romacs_face::BoundedDatum::new(spec).unwrap())
            .unwrap_or_else(|e| panic!("{face}: {e}"));
        if !c.is_uniform() {
            two += 1;
        }
        let p = spec.to_lisp();
        if [
            "(class ",
            "(min-colors ",
            "(type ",
            "(background ",
            "(supports ",
        ]
        .iter()
        .any(|k| p.contains(k))
        {
            conditional += 1;
        }
    }
    eprintln!(
        "{} specs: {conditional} conditional, {two} keep separate light/dark definitions",
        table.len()
    );
    assert!(two > 0 && two < conditional);
}
