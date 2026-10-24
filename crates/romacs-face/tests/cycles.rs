//! Cycles are rejected at construction, never guarded during resolution.
//!
//! GNU 32.0.50, `-Q --batch`, live (transcripts in GNU-DIVERGENCES.md):
//! * a circular face-ref list and a plist whose `:inherit` is itself both
//!   crash `face-attributes-as-vector` with SIGSEGV (exit 139), because
//!   `merge_face_ref` recurses in C without consulting `max-lisp-eval-depth`;
//! * a plist with a circular tail hangs it (killed after 8 s, exit 137).

mod common;

use std::sync::Arc;

use common::*;
use romacs_face::{Background, Datum, DatumError, Deffaces, FaceError, Remap, Theme, View};

fn plain_base() -> romacs_face::Base {
    base_from(&d(
        r#"(:family "Mono" :foundry "x" :width normal :height 100 :weight normal :slant normal
        :underline nil :overline nil :extend nil :strike-through nil :box nil :inverse-video nil
        :foreground "black" :background "white" :stipple nil)"#,
    ))
}

fn deffaces(entries: &[(&str, &str)]) -> Arc<Deffaces> {
    let mut df = Deffaces::default();
    for (name, spec) in entries {
        df.defface(name, &d(spec)).unwrap();
    }
    Arc::new(df)
}

#[test]
fn circular_face_ref_list_is_rejected_at_conversion() {
    // GNU: (let ((l (list 'bold))) (setcdr l l) (face-attributes-as-vector l)) => SIGSEGV
    let mut h = Heap::default();
    let bold = h.atom(Datum::symbol("bold"));
    let l = h.list(&[bold]);
    h.set_cdr(l, l);
    assert_eq!(
        Datum::from_graph(&h, l),
        Err(DatumError::Circular("a list's tail returns to itself"))
    );
}

#[test]
fn circular_inherit_plist_is_rejected_at_conversion() {
    // GNU: (let ((p (list :inherit nil))) (setcar (cdr p) p) (face-attributes-as-vector p)) => SIGSEGV
    let mut h = Heap::default();
    let kw = h.atom(Datum::symbol(":inherit"));
    let nil = h.nil();
    let p = h.list(&[kw, nil]);
    let second = h.cdr(p);
    h.set_car(second, p);
    assert_eq!(
        Datum::from_graph(&h, p),
        Err(DatumError::Circular("a list contains itself"))
    );
}

#[test]
fn circular_plist_tail_is_rejected_at_conversion() {
    // GNU: (let ((p (list :foreground "red"))) (setcdr (cdr p) p) ...) => hangs
    let mut h = Heap::default();
    let kw = h.atom(Datum::symbol(":foreground"));
    let red = h.atom(Datum::string("red"));
    let p = h.list(&[kw, red]);
    let second = h.cdr(p);
    h.set_cdr(second, p);
    let err = Datum::from_graph(&h, p).unwrap_err();
    assert!(matches!(err, DatumError::Circular(_)), "{err}");
    assert_eq!(
        err.to_string(),
        "circular list structure in face spec (a list's tail returns to itself)"
    );
}

#[test]
fn shared_acyclic_structure_is_not_a_cycle() {
    // The same sublist reached twice is a DAG, which GNU and romacs accept.
    let mut h = Heap::default();
    let bold = h.atom(Datum::symbol("bold"));
    let shared = h.list(&[bold]);
    let outer = h.list(&[shared, shared]);
    assert_eq!(
        Datum::from_graph(&h, outer),
        Ok((*d("((bold) (bold))")).clone())
    );
}

#[test]
fn deep_and_long_structure_is_bounded_without_recursion() {
    let result = on_small_stack(|| {
        let mut h = Heap::default();
        // 200k levels of nesting would overflow a 128 KiB stack in any
        // recursive walk; the conversion must refuse it iteratively.
        let mut node = h.nil();
        for _ in 0..200_000 {
            node = h.list(&[node]);
        }
        let deep = Datum::from_graph(&h, node);
        // A 2M-element flat list exceeds the size bound.
        let x = h.atom(Datum::Int(1));
        let mut tail = h.nil();
        for _ in 0..2_000_000 {
            tail = h.cons(x, tail);
        }
        (deep, Datum::from_graph(&h, tail))
    });
    assert_eq!(
        result,
        (Err(DatumError::TooDeep), Err(DatumError::TooLarge))
    );
}

#[test]
fn named_inheritance_cycle_is_rejected_when_the_theme_is_built() {
    // Each defface is fine alone; together they close a loop.
    let df = deffaces(&[
        ("s86-a", "((t :inherit s86-b))"),
        ("s86-b", "((t :inherit s86-c))"),
        ("s86-c", "((t :inherit (bold s86-a)))"),
        ("bold", "((t :weight bold))"),
    ]);
    let err = Theme::build(df, None, Background::Light, plain_base()).unwrap_err();
    let FaceError::InheritanceCycle { cycle } = &err else {
        panic!("{err}")
    };
    assert_eq!(cycle.first(), cycle.last());
    assert_eq!(cycle.len(), 4, "{cycle:?}");
    for f in ["s86-a", "s86-b", "s86-c"] {
        assert!(cycle.iter().any(|c| c == f), "{cycle:?}");
    }
    assert!(
        err.to_string()
            .starts_with("face inheritance results in inheritance cycle: ")
    );
}

#[test]
fn self_inheritance_is_rejected_with_gnus_message() {
    // GNU: (set-face-attribute 's86-a nil :inherit 's86-a)
    //   => (error "Face inheritance results in inheritance cycle" s86-a)
    let df = deffaces(&[("s86-a", "((t))")]);
    let t = Theme::build(df, None, Background::Light, plain_base()).unwrap();
    let err = t
        .with_face_attribute("s86-a", ":inherit", &d("s86-a"))
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "face inheritance results in inheritance cycle: s86-a -> s86-a"
    );
}

#[test]
fn set_face_attribute_closing_a_cycle_leaves_the_old_theme_whole() {
    // GNU: defface a :inherit b; (set-face-attribute 'b nil :inherit 'a) => error
    let df = deffaces(&[
        ("s86-a", "((t :inherit s86-b))"),
        ("s86-b", "((t :foreground \"red\"))"),
    ]);
    let t = Theme::build(df, None, Background::Light, plain_base()).unwrap();
    let before = t.generation();
    let err = t
        .with_face_attribute("s86-b", ":inherit", &d("s86-a"))
        .unwrap_err();
    assert!(matches!(err, FaceError::InheritanceCycle { .. }), "{err}");
    assert_eq!(t.generation(), before);
    assert_eq!(
        attrs_get(
            &t.realize("s86-a").unwrap(),
            romacs_face::Attribute::Foreground
        ),
        d("\"red\"")
    );
}

#[test]
fn long_inheritance_chain_resolves_without_recursion() {
    let (fg, len) = on_small_stack(|| {
        let mut df = Deffaces::default();
        df.defface("f0", &d("((t :foreground \"red\"))")).unwrap();
        for i in 1..20_000 {
            df.defface(
                &format!("f{i}"),
                &romacs_face::BoundedDatum::parse(&format!("((t :inherit f{}))", i - 1)).unwrap(),
            )
            .unwrap();
        }
        let t = Theme::build(Arc::new(df), None, Background::Light, plain_base()).unwrap();
        (
            attrs_get(
                &t.realize("f19999").unwrap(),
                romacs_face::Attribute::Foreground,
            ),
            t.len(),
        )
    });
    assert_eq!((fg, len), ((*d("\"red\"")).clone(), 20_000));
}

#[test]
fn remapping_that_closes_a_loop_is_rejected_at_binding() {
    // Reject a cycle between two remapping entries. Inheritance backrefs
    // alone are valid and covered by the GNU C parity fixture.
    let df = deffaces(&[
        ("s86-a", "((t :inherit s86-b))"),
        ("s86-b", "((t :foreground \"red\"))"),
    ]);
    let t = Theme::build(df, None, Background::Light, plain_base()).unwrap();
    let remap = Remap::from_alist(&d("((s86-a s86-b) (s86-b s86-a))")).unwrap();
    let err = View::bind(&t, &remap).unwrap_err();
    let FaceError::InheritanceCycle { cycle } = err else {
        panic!()
    };
    assert!(
        cycle.contains(&"s86-a".to_owned()) && cycle.contains(&"s86-b".to_owned()),
        "{cycle:?}"
    );
    // A face remapped onto itself plus attributes is not a loop.
    let ok = Remap::from_alist(&d("((s86-b (:weight bold) s86-b))")).unwrap();
    let v = View::bind(&t, &ok).unwrap();
    let a = v.realize("s86-a").unwrap();
    assert_eq!(attrs_get(&a, romacs_face::Attribute::Weight), d("bold"));
    assert_eq!(
        attrs_get(&a, romacs_face::Attribute::Foreground),
        d("\"red\"")
    );
}

#[test]
fn palette_mapping_loop_is_rejected_at_construction() {
    use romacs_face::palette::parse_layer;
    let layer = parse_layer(&d("((a b) (b c) (c a) (d \"#fff\") (e d) (f nowhere))")).unwrap();
    let err = romacs_face::Palette::layered(&[&layer]).unwrap_err();
    assert_eq!(err.to_string(), "palette mapping loop: a -> b -> c -> a");
    let ok = parse_layer(&d("((d \"#fff\") (e d) (f nowhere) (g unspecified))")).unwrap();
    let p = romacs_face::Palette::layered(&[&ok]).unwrap();
    assert_eq!(
        (p.get("e"), p.get("f"), p.get("g"), p.get("zz")),
        (Some(Some("#fff")), Some(None), Some(None), None)
    );
}
