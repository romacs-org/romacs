//! A walk of listed values (`Walk`): what it counts, and the layouts it
//! records, against what the compiler says.
use romacs_snapshot::{EMPTY_HASH_TABLE, Notes, Walk, snapshot_enum, snapshot_struct, snapshot_tuple};
use serde_json::Value as Json;
use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Point(u32, u64);
snapshot_tuple!(Point(x, y));

pub enum Shape {
    Dot,
    Line(Point),
    Many(Vec<Shape>),
}
snapshot_enum!(Shape { Shape::Dot => (), Shape::Line(point) => (point), Shape::Many(shapes) => (shapes) });

pub struct Scene {
    name: String,
    shapes: Vec<Shape>,
    shared: (Rc<Point>, Rc<Point>),
    index: HashMap<u32, u32>,
    seen: u8,
}
snapshot_struct!(Scene { name, shapes, shared, index } at_rest { seen });

fn scene() -> Scene {
    let point = Rc::new(Point(1, 2));
    let shapes = vec![Shape::Dot, Shape::Line(Point(3, 4)), Shape::Many(vec![Shape::Line(Point(5, 6)), Shape::Dot])];
    Scene { name: "scene".to_owned(), shapes, shared: (point.clone(), point), index: HashMap::new(), seen: 0 }
}

/// The census of a walk from SCENE, with every Point the observer saw.
fn census(scene: &Scene) -> (Json, Vec<u64>) {
    let mut ys = Vec::new();
    let mut observe = |value: &dyn Any, notes: &mut Notes| {
        if let Some(point) = value.downcast_ref::<Point>() {
            ys.push(point.1);
            notes.add("points", 1);
        }
    };
    let mut walk = Walk::new(&mut observe);
    walk.value(scene);
    let census = walk.census();
    (census, ys)
}

fn kind<'a>(census: &'a Json, name: &str) -> &'a Json {
    census["kinds"].as_array().unwrap().iter().find(|kind| kind["type"] == name).unwrap_or_else(|| panic!("no kind {name}"))
}

#[test]
fn a_walk_counts_each_value_once_and_notes_what_it_finds() {
    let scene = scene();
    let (census, ys) = census(&scene);
    assert_eq!(kind(&census, "walk::Scene")["count"], 1);
    // Three points inline, one shared twice and walked once.
    assert_eq!(kind(&census, "walk::Point")["count"], 3);
    assert_eq!(ys, [4, 6, 2]);
    assert_eq!(census["notes"]["points"], 3);
    assert_eq!(census["totals"]["shared"], 1);
    // An empty hash table points into the executable.
    assert_eq!(census["notes"][EMPTY_HASH_TABLE], 1);
    // The at-rest field is not walked: no u8 met.
    assert!(census["kinds"].as_array().unwrap().iter().all(|kind| kind["type"] != "u8"));
    // Each variant counted.
    let shapes = &kind(&census, "walk::Shape")["layouts"];
    let count = |variant: &str| shapes[variant]["count"].as_u64();
    assert_eq!(kind(&census, "walk::Shape")["count"], 5);
    assert_eq!((count("Shape::Dot"), count("Shape::Line(point)"), count("Shape::Many(shapes)")), (Some(2), Some(2), Some(1)));
    // The text's allocation is the String's.
    assert_eq!(kind(&census, "alloc::string::String")["owned_bytes"], scene.name.capacity());
}

#[test]
fn a_layout_is_the_compilers() {
    let (census, _) = census(&scene());
    let fields = |kind_name: &str, variant: &str| -> Vec<(String, u64, u64)> {
        kind(&census, kind_name)["layouts"][variant]["fields"].as_array().unwrap().iter()
            .map(|f| (f[0].as_str().unwrap().to_owned(), f[2].as_u64().unwrap(), f[3].as_u64().unwrap())).collect()
    };
    let at = |offset: usize, size: usize| (offset as u64, size as u64);
    let scene_fields = fields("walk::Scene", "");
    let expect = [("name", at(std::mem::offset_of!(Scene, name), size_of::<String>())),
                  ("shapes", at(std::mem::offset_of!(Scene, shapes), size_of::<Vec<Shape>>())),
                  ("shared", at(std::mem::offset_of!(Scene, shared), size_of::<(Rc<Point>, Rc<Point>)>())),
                  ("index", at(std::mem::offset_of!(Scene, index), size_of::<HashMap<u32, u32>>()))];
    assert_eq!(scene_fields, expect.map(|(name, (offset, size))| (name.to_owned(), offset, size)));
    assert_eq!(fields("walk::Point", ""), [("x".to_owned(), std::mem::offset_of!(Point, 0) as u64, 4), ("y".to_owned(), std::mem::offset_of!(Point, 1) as u64, 8)]);
    // A variant's field lies inside the enum, once, though a Many holds
    // more Lines.
    let line = fields("walk::Shape", "Shape::Line(point)");
    assert_eq!(line.len(), 1);
    assert!(line[0].1 + line[0].2 <= size_of::<Shape>() as u64, "{line:?}");
}

pub struct Cells {
    held: std::cell::RefCell<u32>,
    boxed: Box<[u64; 4]>,
}
snapshot_struct!(Cells { held, boxed });

#[test]
fn a_value_inside_its_own_variant_describes_it_once_and_a_taken_cell_is_noted() {
    // A Many inside the first Many met: the outer one describes the
    // variant, and the inner one adds no field.
    let shapes = Shape::Many(vec![Shape::Many(vec![])]);
    let mut observe = |_: &dyn Any, _: &mut Notes| {};
    let mut walk = Walk::new(&mut observe);
    walk.value(&shapes);
    let census = walk.census();
    let many = &kind(&census, "walk::Shape")["layouts"]["Shape::Many(shapes)"];
    assert_eq!((many["count"].as_u64(), many["fields"].as_array().map(Vec::len)), (Some(2), Some(1)));

    // A cell borrowed while the walk runs is noted, not read; a box owns
    // its value's bytes.
    let cells = Cells { held: std::cell::RefCell::new(7), boxed: Box::new([0; 4]) };
    let borrowed = cells.held.borrow_mut();
    let mut walk = Walk::new(&mut observe);
    walk.value(&cells);
    let census = walk.census();
    drop(borrowed);
    assert_eq!(census["notes"][romacs_snapshot::TAKEN], 1);
    assert_eq!(kind(&census, "alloc::boxed::Box<[u64; 4]>")["owned_bytes"], 32);
}
