//! A walk of listed values (`Walk`): what it counts, and the layouts it
//! records, against what the compiler says.
use romacs_snapshot::{EMPTY_HASH_TABLE, Notes, Walk};
use serde_json::Value as Json;
use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(romacs_snapshot::Snapshot)]
pub struct Point(u32, u64);

#[derive(romacs_snapshot::Snapshot)]
pub enum Shape {
    Dot,
    Line(Point),
    Many(Vec<Shape>),
}

#[derive(romacs_snapshot::Snapshot)]
#[snapshot(root)]
pub struct Scene {
    name: String,
    shapes: Vec<Shape>,
    shared: (Rc<Point>, Rc<Point>),
    index: HashMap<u32, u32>,
    #[snapshot(at_rest)]
    seen: u8,
}

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
    assert_eq!((count("Dot"), count("Line"), count("Many")), (Some(2), Some(2), Some(1)));
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
    assert_eq!(fields("walk::Point", ""), [("0".to_owned(), std::mem::offset_of!(Point, 0) as u64, 4), ("1".to_owned(), std::mem::offset_of!(Point, 1) as u64, 8)]);
    // A variant's field lies inside the enum, once, though a Many holds
    // more Lines.
    let line = fields("walk::Shape", "Line");
    assert_eq!(line.len(), 1);
    assert!(line[0].1 + line[0].2 <= size_of::<Shape>() as u64, "{line:?}");
}

#[derive(romacs_snapshot::Snapshot)]
pub struct Cells {
    held: std::cell::RefCell<u32>,
    boxed: Box<[u64; 4]>,
}

#[test]
fn a_value_inside_its_own_variant_describes_it_once_and_a_taken_cell_is_noted() {
    // A Many inside the first Many met: the outer one describes the
    // variant, and the inner one adds no field.
    let shapes = Shape::Many(vec![Shape::Many(vec![])]);
    let mut observe = |_: &dyn Any, _: &mut Notes| {};
    let mut walk = Walk::new(&mut observe);
    walk.value(&shapes);
    let census = walk.census();
    let many = &kind(&census, "walk::Shape")["layouts"]["Many"];
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

/// A table of tables.
type Nested = HashMap<u32, Vec<HashMap<u32, u32>>>;

#[derive(romacs_snapshot::Snapshot)]
pub struct Tables {
    plain: HashMap<u32, u32>,
    nested: Vec<std::collections::HashSet<u32>>,
    owned: Rc<HashMap<u32, u32>>,
    boxed: Box<[Nested; 1]>,
}

fn empty_tables(value: &Tables) -> u64 {
    let mut observe = |_: &dyn Any, _: &mut Notes| {};
    let mut walk = Walk::new(&mut observe);
    walk.value(value);
    walk.noted(EMPTY_HASH_TABLE)
}

#[test]
fn settling_gives_each_empty_table_in_reach_its_own_allocation() {
    use romacs_snapshot::{Schema, Settle};
    let mut boxed = HashMap::new();
    boxed.insert(1, vec![HashMap::new()]);
    let mut tables = Tables { plain: HashMap::new(), nested: vec![std::collections::HashSet::new(); 2], owned: Rc::new(HashMap::new()),
                              boxed: Box::new([boxed]) };
    assert_eq!(empty_tables(&tables), 5);
    let mut settle = Settle::default();
    tables.settle(&mut settle);
    assert_eq!((settle.settled(), settle.unsettled()), (5, &[][..]));
    // Each is empty still, and no longer points into the executable.
    assert!(tables.plain.is_empty() && tables.plain.capacity() > 0);
    assert!(tables.nested.iter().all(|set| set.is_empty() && set.capacity() > 0));
    assert_eq!(empty_tables(&tables), 0);
}

#[test]
fn an_empty_table_another_owner_shares_is_named_not_changed() {
    use romacs_snapshot::{Schema, Settle};
    let shared = Rc::new(HashMap::new());
    let mut tables = Tables { plain: HashMap::from([(1, 1)]), nested: vec![], owned: shared.clone(), boxed: Box::new([HashMap::from([(1, vec![])])]) };
    let mut settle = Settle::default();
    tables.settle(&mut settle);
    assert_eq!(settle.unsettled(), ["std::collections::hash::map::HashMap<u32, u32>"]);
    assert_eq!(shared.capacity(), 0);
}

/// A hasher that holds a table of its own.
#[derive(Default, romacs_snapshot::Snapshot)]
pub struct TableHasher {
    seen: HashMap<u32, u32>,
}
impl std::hash::BuildHasher for TableHasher {
    type Hasher = std::collections::hash_map::DefaultHasher;
    fn build_hasher(&self) -> Self::Hasher {
        std::collections::hash_map::DefaultHasher::new()
    }
}

/// An element whose ordering ignores the table it holds.
#[derive(romacs_snapshot::Snapshot)]
pub struct Ranked {
    rank: u32,
    cache: std::cell::RefCell<HashMap<u32, u32>>,
}
impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.rank == other.rank
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank.cmp(&other.rank)
    }
}

#[derive(romacs_snapshot::Snapshot)]
pub struct Unreachable {
    hashed: HashMap<u32, u32, TableHasher>,
    heap: std::collections::BinaryHeap<Ranked>,
    locked: std::sync::Mutex<HashMap<u32, u32>>,
    borrowed: Rc<std::cell::RefCell<HashMap<u32, u32>>>,
}

/// What settling cannot reach is checked: a hasher's table, a heap element's,
/// and a shared cell that is borrowed are named. A poisoned lock's table is
/// reached all the same.
#[test]
fn settling_names_what_it_cannot_reach_and_recovers_a_poisoned_lock() {
    use romacs_snapshot::{Schema, Settle};
    let mut hashed = HashMap::with_hasher(TableHasher::default());
    hashed.insert(1, 1);
    let heap = std::collections::BinaryHeap::from([Ranked { rank: 1, cache: std::cell::RefCell::new(HashMap::new()) }]);
    let locked = std::sync::Mutex::new(HashMap::new());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = locked.lock().unwrap();
        panic!("poison the lock");
    }));
    assert!(locked.is_poisoned());
    let borrowed = Rc::new(std::cell::RefCell::new(HashMap::from([(1, 1)])));
    let mut value = Unreachable { hashed, heap, locked, borrowed: borrowed.clone() };
    let held = borrowed.borrow_mut();
    let mut settle = Settle::default();
    value.settle(&mut settle);
    drop(held);
    assert_eq!(settle.unsettled(), ["walk::TableHasher", "walk::Ranked", "core::cell::RefCell<std::collections::hash::map::HashMap<u32, u32>>"]);
    assert_eq!(settle.settled(), 1, "the poisoned lock's table");
    assert!(value.locked.lock().unwrap_or_else(std::sync::PoisonError::into_inner).capacity() > 0);
}

/// A variant whose fields are named as the derive's own arguments.
#[derive(romacs_snapshot::Snapshot)]
pub enum Named {
    Pair { w: u32, s: HashMap<u32, u32> },
}

/// A struct variant's fields are walked and settled under names of the
/// derive's own, whatever the fields are called.
#[test]
fn a_variants_fields_are_met_whatever_their_names() {
    use romacs_snapshot::{Schema, Settle};
    let mut value = Named::Pair { w: 1, s: HashMap::new() };
    let mut observe = |_: &dyn Any, _: &mut Notes| {};
    let mut walk = Walk::new(&mut observe);
    walk.value(&value);
    let census = walk.census();
    let fields: Vec<_> = kind(&census, "walk::Named")["layouts"]["Pair"]["fields"].as_array().unwrap().iter().map(|f| f[0].clone()).collect();
    assert_eq!(fields, ["w", "s"]);
    let mut settle = Settle::default();
    value.settle(&mut settle);
    assert_eq!(settle.settled(), 1);
}
