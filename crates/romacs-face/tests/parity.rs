//! Attribute-by-attribute parity with live GNU Emacs for every bundled theme.
//!
//! Fixtures: tests/gnu/fixtures, produced by tests/gnu/regenerate.sh from
//! `$ROMACS_GNU_ORACLE -Q --batch` (GNU 32.0.50 at the pin, see fixtures/ORACLE) with GNU's display predicates
//! told "pgtk, truecolour, colour class", and nothing else simulated; see
//! tests/gnu/dump.el for the regime and its limits.
//!
//! Two comparisons per face and attribute:
//! * own: the value after spec collapse and theme application, before
//!   inheritance (GNU `face-attribute FACE ATTR nil nil`). This is the
//!   *declared* value, independent of any frame.
//! * realized: merged along `:inherit` and over the default face (GNU
//!   `face-attribute FACE ATTR nil 'default`).
//!
//! A batch frame has no font or colours of its own, so a realized value that
//! GNU took from the frame ("unspecified-fg", family "default", height 1) is
//! a placeholder: matching it proves plumbing, not resolution. Those are
//! counted apart, and the test requires that most realized comparisons are
//! values a theme or defface supplied.

mod common;

use common::*;
use romacs_face::{Background, Datum, Inherit, Palette, Theme};

#[derive(Default, Debug)]
struct Tally {
    own: usize,
    realized: usize,
    realized_from_frame: usize,
    realized_from_specs: usize,
    /// GNU-DIVERGENCES.md "reset is resolved": GNU's Lisp `face-attribute`
    /// returns the symbol `reset`, while its own C merge (and redisplay)
    /// substitutes the default face's value. romacs returns the latter.
    reset_resolved: usize,
    mismatches: Vec<String>,
}

fn inherit_names(d: &Datum) -> Vec<String> {
    match d {
        Datum::Nil => vec![],
        Datum::List(v) => v.iter().map(|x| x.to_lisp()).collect(),
        other => vec![other.to_lisp()],
    }
}

/// Values a batch frame supplies because it has no font or colours.
fn is_frame_placeholder(a: romacs_face::Attribute, v: &Datum) -> bool {
    use romacs_face::Attribute as A;
    match (a, v) {
        (A::Foreground | A::Background, Datum::Str(s)) => s.starts_with("unspecified-"),
        (A::Family | A::Foundry, Datum::Str(s)) => &**s == "default",
        (A::Height, Datum::Int(1)) => true,
        _ => false,
    }
}

fn compare(dump: &Dump, table: &[(String, Datum)], name: &str) -> Tally {
    let (deffaces, spec) = inputs(dump, table);
    let theme = Theme::build(deffaces, spec, dump.mode, base_from(&dump.base))
        .unwrap_or_else(|e| panic!("{name}: theme rejected: {e}"));
    let mut t = Tally::default();
    for (face, _, own, realized) in &dump.faces {
        let def = theme
            .definition(face)
            .unwrap_or_else(|| panic!("{name}: no face {face}"));
        // :inherit, declared.
        let gnu_inherit = plist_get(own, ":inherit")
            .cloned()
            .unwrap_or(Datum::symbol("unspecified"));
        let ours_inherit = match def.inherit() {
            None => vec!["unspecified".to_owned()],
            Some(v) => v.iter().map(|s| s.to_string()).collect(),
        };
        let gnu_names = inherit_names(&gnu_inherit);
        t.own += 1;
        if gnu_names != ours_inherit {
            t.mismatches.push(format!(
                "{name}/{face} own :inherit gnu={gnu_inherit} ours={ours_inherit:?}"
            ));
        }
        let ours_realized = theme.realize(face).expect("face exists");
        for a in dumped_attributes() {
            let gnu = plist_get(own, a.keyword())
                .cloned()
                .unwrap_or(Datum::symbol("unspecified"));
            // GNU's default face is the frame's face: its own slots are
            // already absolute, so compare it with the realised record.
            let ours = if face == "default" {
                attrs_get(&ours_realized, a)
            } else {
                slot_datum(&theme.face_attribute(face, a, Inherit::No).expect("face"))
            };
            t.own += 1;
            if gnu != ours {
                t.mismatches.push(format!(
                    "{name}/{face} own {} gnu={gnu} ours={ours}",
                    a.keyword()
                ));
            }
            let gnu = plist_get(realized, a.keyword())
                .or_else(|| plist_get(&dump.default_row, a.keyword()))
                .cloned()
                .expect("default row is complete");
            let ours = attrs_get(&ours_realized, a);
            t.realized += 1;
            // Face-specific: GNU's value differs from the realised default
            // face, so the face's own spec or its inheritance produced it.
            if plist_get(realized, a.keyword()).is_some() {
                t.realized_from_specs += 1;
            }
            if is_frame_placeholder(a, &gnu) {
                t.realized_from_frame += 1;
            }
            let default_value = plist_get(&dump.default_row, a.keyword()).expect("row");
            if gnu.as_symbol() == Some("reset") {
                if ours == *default_value {
                    t.reset_resolved += 1;
                } else {
                    t.mismatches.push(format!("{name}/{face} realized {} reset not resolved to the default face: ours={ours}", a.keyword()));
                }
            } else if gnu != ours {
                t.mismatches.push(format!(
                    "{name}/{face} realized {} gnu={gnu} ours={ours}",
                    a.keyword()
                ));
            }
        }
    }
    t
}

#[test]
fn every_bundled_theme_matches_gnu_attribute_by_attribute() {
    let table = deffaces_table();
    let names = theme_names();
    assert_eq!(
        names.len(),
        25,
        "expected the 25 oracle theme dumps in tests/gnu/fixtures: {names:?}"
    );
    let mut all = Vec::new();
    let (mut own, mut realized, mut specs, mut frame) = (0, 0, 0, 0);
    for name in std::iter::once("nil".to_owned()).chain(names) {
        let dump = load_dump(&name);
        assert!(
            dump.faces.len() > 400,
            "{name}: only {} faces",
            dump.faces.len()
        );
        let t = compare(&dump, &table, &name);
        eprintln!(
            "{name:28} faces={:4} own={:5} realized={:5} (face-specific {:5}, frame placeholders {:5}) reset-resolved={} mismatches={}",
            dump.faces.len(),
            t.own,
            t.realized,
            t.realized_from_specs,
            t.realized_from_frame,
            t.reset_resolved,
            t.mismatches.len()
        );
        // newcomers-presets sets variables and a few faces, not colours.
        if name != "nil" && name != "newcomers-presets" {
            // A theme colours the default face, so its colours are real.
            assert!(
                !is_frame_placeholder(
                    romacs_face::Attribute::Foreground,
                    plist_get(&dump.default_row, ":foreground").unwrap()
                ),
                "{name}: default foreground is a batch placeholder; colour parity would prove nothing"
            );
            assert!(
                t.realized_from_specs > 300,
                "{name}: only {} face-specific values",
                t.realized_from_specs
            );
        }
        own += t.own;
        realized += t.realized;
        specs += t.realized_from_specs;
        frame += t.realized_from_frame;
        all.extend(t.mismatches);
    }
    eprintln!(
        "TOTAL own={own} realized={realized} face-specific={specs} frame-placeholders={frame} mismatches={}",
        all.len()
    );
    for m in all.iter().take(400) {
        eprintln!("  {m}");
    }
    assert!(all.is_empty(), "{} mismatches with GNU", all.len());
}

#[test]
fn the_parity_comparison_can_fail() {
    // A wrong colour in one theme spec must surface as exactly that mismatch,
    // or the parity numbers above would mean nothing.
    let table = deffaces_table();
    let mut dump = load_dump("tango-dark");
    let entry = dump
        .faces
        .iter_mut()
        .find(|(f, ..)| f == "font-lock-comment-face")
        .expect("face");
    let spec = entry.1.take().expect("tango-dark styles comments");
    let printed = spec.to_lisp();
    assert!(printed.contains("#73d216"), "{printed}");
    entry.1 = Some(Datum::parse(&printed.replace("#73d216", "#000001")).unwrap());
    let baseline = compare(&load_dump("tango-dark"), &table, "tango-dark");
    let t = compare(&dump, &table, "tango-dark");
    let new: Vec<&String> = t
        .mismatches
        .iter()
        .filter(|m| !baseline.mismatches.contains(m))
        .collect();
    let own = "tango-dark/font-lock-comment-face own :foreground gnu=\"#73d216\" ours=\"#000001\"";
    let realized =
        "tango-dark/font-lock-comment-face realized :foreground gnu=\"#73d216\" ours=\"#000001\"";
    assert!(
        new.iter().any(|m| *m == own) && new.iter().any(|m| *m == realized),
        "{new:#?}"
    );
    // Faces inheriting the comment face see the wrong colour too, and
    // nothing else moves.
    let wrong = "realized :foreground gnu=\"#73d216\" ours=\"#000001\"";
    assert!(
        new.iter().all(|m| *m == own || m.ends_with(wrong)),
        "{new:#?}"
    );
    eprintln!("one mutated colour produced {} mismatches", new.len());
}

#[test]
fn background_mode_is_derived_like_gnu() {
    for name in theme_names() {
        let dump = load_dump(&name);
        let Some(Datum::Str(bg)) = plist_get(&dump.default_row, ":background") else {
            continue;
        };
        if let Some(mode) = Background::of_hex_color(bg) {
            assert_eq!(mode, dump.mode, "{name}: default background {bg}");
        }
    }
    // Themes' backgrounds are far from the threshold; GNU's `color-dark-p'
    // over a sweep that crosses it pins the arithmetic.
    let cases = read_datum(&fixtures().join("remap.eld"));
    let sweep = cases
        .as_list()
        .unwrap()
        .iter()
        .map(|c| c.as_list().unwrap())
        .find(|c| symbol(&c[0]) == "dark-p")
        .unwrap();
    let mut n = 0;
    for pair in sweep[1].as_list().unwrap() {
        // (COLOR . t) is dark; (COLOR . nil) prints as (COLOR).
        let (color, gnu) = match pair {
            Datum::Dotted(k, _) => (&k[0], Background::Dark),
            Datum::List(k) if k.len() == 1 => (&k[0], Background::Light),
            other => panic!("{other}"),
        };
        let Datum::Str(color) = color else { panic!() };
        assert_eq!(Background::of_hex_color(color), Some(gnu), "{color}");
        n += 1;
    }
    assert!(n > 60);
}

#[test]
fn modus_palettes_resolve_like_modus_including_rum() {
    let mut checked = 0;
    let mut themes = 0;
    for name in theme_names() {
        let dump = load_dump(&name);
        if dump.palette.is_nil() {
            continue;
        }
        themes += 1;
        let layer = |k: &str| {
            romacs_face::palette::parse_layer(
                &romacs_face::BoundedDatum::new(plist_get(&dump.palette, k).unwrap_or(&Datum::Nil))
                    .unwrap(),
            )
            .unwrap()
        };
        let (o, u, c) = (layer(":overrides"), layer(":user"), layer(":core"));
        let p = Palette::layered(&[&o, &u, &c]).unwrap_or_else(|e| panic!("{name}: {e}"));
        for entry in plist_get(&dump.palette, ":resolved")
            .and_then(Datum::as_list)
            .expect("resolved")
        {
            let v = entry.as_list().unwrap();
            let key = symbol(&v[0]);
            let gnu = match &v[1] {
                Datum::Str(s) => Some(&**s),
                Datum::Symbol(s) if &**s == "unspecified" => None,
                other => panic!("{name}/{key}: {other}"),
            };
            assert_eq!(p.get(key), Some(gnu), "{name}: palette entry {key}");
            checked += 1;
        }
    }
    eprintln!("palette entries checked: {checked} across {themes} themes");
    assert!(themes >= 9, "modus family plus rum expected, got {themes}");
}
