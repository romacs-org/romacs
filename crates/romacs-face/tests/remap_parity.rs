//! `face-remapping-alist` and anonymous face references against GNU's C
//! merge (`face-attributes-as-vector`, merge_face_ref with no window), live
//! GNU 32.0.50; fixture tests/gnu/fixtures/remap.eld from tests/gnu/remap.el.

mod common;

use std::sync::Arc;

use common::*;
use romacs_face::remap::parse_face_ref;
use romacs_face::{
    Attribute, Attrs, Background, Datum, Deffaces, FaceError, Remap, Slot, Theme, View,
};

fn plain_base() -> romacs_face::Base {
    base_from(&d(
        r#"(:family "Mono" :foundry "x" :width normal :height 100 :weight normal :slant normal
        :underline nil :overline nil :extend nil :strike-through nil :box nil :inverse-video nil
        :foreground "black" :background "white" :stipple nil)"#,
    ))
}

fn specified(attrs: &Attrs) -> Vec<(Attribute, Datum)> {
    Attribute::ALL
        .into_iter()
        .filter(|a| !matches!(attrs.get(*a), Slot::Unspecified))
        .map(|a| (a, attrs_get(attrs, a)))
        .collect()
}

fn gnu_specified(plist: &Datum) -> Vec<(Attribute, Datum)> {
    let mut v: Vec<(Attribute, Datum)> = plist
        .as_list()
        .unwrap()
        .chunks(2)
        .map(|kv| {
            (
                Attribute::from_keyword(symbol(&kv[0])).unwrap(),
                kv[1].clone(),
            )
        })
        .collect();
    v.sort_by_key(|(a, _)| *a);
    v
}

/// Ours for one case: Ok(specified attributes) or the binding error.
fn ours(case: &[Datum]) -> Result<Vec<(Attribute, Datum)>, FaceError> {
    let mut df = Deffaces::default();
    df.defface("default", &d("((t nil))")).unwrap();
    for f in case[1].as_list().unwrap() {
        let f = f.as_list().unwrap();
        df.defface(
            symbol(&f[0]),
            &romacs_face::BoundedDatum::new(&f[1]).unwrap(),
        )
        .unwrap();
    }
    let theme = Theme::build(Arc::new(df), None, Background::Light, plain_base()).unwrap();
    let remap = if case[2].is_nil() {
        Remap::empty()
    } else {
        Remap::from_alist(&romacs_face::BoundedDatum::new(&case[2]).unwrap())?
    };
    let view = View::bind(&theme, &remap)?;
    let attrs = match &case[3] {
        Datum::Symbol(name) => view.effective(name).unwrap(),
        r => view
            .effective_ref(&parse_face_ref(&romacs_face::BoundedDatum::new(r).unwrap()).unwrap()),
    };
    Ok(specified(&attrs))
}

#[test]
fn remapping_and_anonymous_faces_match_gnu() {
    let cases = read_datum(&fixtures().join("remap.eld"));
    let cases: Vec<&Datum> = cases
        .as_list()
        .unwrap()
        .iter()
        .filter(|c| !["heights", "dark-p"].contains(&symbol(&c.as_list().unwrap()[0])))
        .collect();
    assert_eq!(cases.len(), 9);
    let mut matched = 0;
    for case in cases {
        let case = case.as_list().unwrap();
        let name = symbol(&case[0]);
        let gnu = gnu_specified(&case[4]);
        let ours = ours(case);
        match name {
            // GNU-DIVERGENCES.md "anonymous plist attributes win over :inherit":
            // GNU merges :inherit where it appears, so an earlier attribute loses.
            "anonymous-attribute-before-inherit" => {
                assert_eq!(gnu, vec![(Attribute::Foreground, (*d("\"blue\"")).clone())]);
                assert_eq!(
                    ours.unwrap(),
                    vec![(Attribute::Foreground, (*d("\"red\"")).clone())]
                );
            }
            _ => {
                assert_eq!(ours.unwrap(), gnu, "case {name}");
                matched += 1;
            }
        }
    }
    assert_eq!(matched, 8);
}

#[test]
fn relative_heights_truncate_like_gnu() {
    // GNU face-attribute ... 'default over defaults of 100, 113, 97 and 1.
    let cases = read_datum(&fixtures().join("remap.eld"));
    let case = cases
        .as_list()
        .unwrap()
        .iter()
        .map(|c| c.as_list().unwrap())
        .find(|c| symbol(&c[0]) == "heights")
        .unwrap();
    let mut df = Deffaces::default();
    df.defface("default", &d("((t nil))")).unwrap();
    let faces: Vec<&str> = case[1]
        .as_list()
        .unwrap()
        .iter()
        .map(|f| symbol(&f.as_list().unwrap()[0]))
        .collect();
    for f in case[1].as_list().unwrap() {
        let f = f.as_list().unwrap();
        df.defface(
            symbol(&f[0]),
            &romacs_face::BoundedDatum::new(&f[1]).unwrap(),
        )
        .unwrap();
    }
    let theme = Theme::build(Arc::new(df), None, Background::Light, plain_base()).unwrap();
    let mut checked = 0;
    for row in case[2].as_list().unwrap() {
        let row = row.as_list().unwrap();
        let t = theme
            .with_face_attribute(
                "default",
                ":height",
                &romacs_face::BoundedDatum::new(&row[0]).unwrap(),
            )
            .unwrap();
        for (face, gnu) in faces.iter().zip(&row[1..]) {
            assert_eq!(
                slot_datum(
                    &t.face_attribute(face, Attribute::Height, romacs_face::Inherit::Default)
                        .unwrap()
                ),
                *gnu,
                "{face} over default {}",
                row[0]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 24);
}

/// A `:filtered` remapping is its face where the window's parameter is the
/// filter's value and nothing elsewhere (xfaces.c filter_face_ref,
/// evaluate_face_filter), and names the parameter it tests.
#[test]
fn filtered_remapping_follows_the_window() {
    let mut df = Deffaces::default();
    df.defface("default", &d("((t nil))")).unwrap();
    let theme = Theme::build(Arc::new(df), None, Background::Light, plain_base()).unwrap();
    let remap = Remap::from_alist(&romacs_face::BoundedDatum::new(&d(
        "((default (:filtered (:window wide t) (:height 2.0)) default))",
    )).unwrap()).unwrap();
    assert_eq!(remap.filter_parameters().iter().map(|p| &**p).collect::<Vec<_>>(), ["wide"]);
    let height = |matches: &dyn Fn(&str, &Datum) -> bool| {
        let view = View::bind(&theme, &remap.for_window(matches)).unwrap();
        attrs_get(&view.realize("default").unwrap(), Attribute::Height)
    };
    assert_eq!(height(&|name, value| name == "wide" && *value == Datum::T), d("200"));
    assert_eq!(height(&|_, _| false), d("100"));
}
