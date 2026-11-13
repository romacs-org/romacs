use super::*;

#[test]
fn keymap_query_preserves_the_threaded_register_allocation() {
    let mut vm = Vm::new();
    let buffer = vm.allocate_object(Object::Buffer(Box::new(
        text::Text::new(b"").unwrap().into(),
    )));
    vm.slots[vm.buffer_slot].value = buffer;
    let tag = Value::reference(vm.slot("keymap") + 1, 7);
    let map = vm.list_value(&[tag]);
    vm.annotated_mut(buffer).unwrap().local_keymap = map;
    let property = Value::reference(vm.slot("local-map") + 1, 7);
    vm.stack.clear();
    vm.stack.shrink_to_fit();
    let registers = vm.stack.as_ptr();
    let capacity = vm.stack.capacity();
    assert!(vm.keymap_at_point(property).is_ok_and(|found| found == map));
    // A native leaf call cannot invalidate the interpreter's cached registers.
    assert_eq!(vm.stack.as_ptr(), registers);
    assert_eq!(vm.stack.capacity(), capacity);
    assert!(vm.stack.is_empty());
}

#[test]
fn mixed_buffer_anchor_coordinates_across_directory_boundaries() {
    let source = "English Ελληνικά Кириллица\n".repeat(600);
    let mut buffer: annotations::AnnotatedBuffer =
        text::Text::new(source.as_bytes()).unwrap().into();
    let mut expected: Vec<_> = (0..buffer.len_chars())
        .step_by(5)
        .map(|p| (p, if p & 1 == 0 { Bias::Left } else { Bias::Right }))
        .collect();
    let handles: Vec<_> = expected
        .iter()
        .map(|&(p, b)| {
            let byte = buffer.byte_of_char(CharOffset(p)).unwrap();
            buffer.anchors.create(byte.0 as u32, b).unwrap()
        })
        .collect();
    for step in 0..80 {
        let a = 480 + step * 3;
        let b = a + 35;
        let from = buffer.byte_of_char(CharOffset(a)).unwrap();
        let to = buffer.byte_of_char(CharOffset(b)).unwrap();
        buffer.replace(from..to, "λEnglishЮ".as_bytes()).unwrap();
        let inserted = "λEnglishЮ".chars().count();
        for (pos, bias) in &mut expected {
            if *pos > a {
                *pos = if *pos <= b { a } else { *pos - (b - a) };
            }
            if *pos > a || *pos == a && *bias == Bias::Right {
                *pos += inserted;
            }
        }
        for (&h, &(pos, _)) in handles.iter().zip(&expected) {
            let byte = buffer.anchors.resolve(h).unwrap();
            assert_eq!(
                buffer.char_of_byte(ByteOffset(byte as usize)).unwrap().0,
                pos,
                "step={step} handle={h:?}"
            );
        }
    }
}
#[test]
fn annotation_graphs_compact_and_dead_marker_slots_are_reused() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    assert_eq!(
        size_of::<Object>(),
        32,
        "annotation variants must not inflate every heap object"
    );
    eprintln!(
        "anchor slot={} marker={} overlay={} object={}",
        anchors::AnchorTree::bytes_per_slot(),
        size_of::<Marker>(),
        size_of::<Overlay>(),
        size_of::<Object>()
    );
    vm.allocate_object(Object::String(Box::new(data::LispString::new("dead"))));
    let buffer = vm.allocate_object(Object::Buffer(Box::new(
        text::Text::new("aλЖz".as_bytes()).unwrap().into(),
    )));
    let anchor = vm
        .annotated_mut(buffer)
        .unwrap()
        .anchors
        .create(1, Bias::Right)
        .unwrap();
    let marker = vm.allocate_object(Object::Marker(Box::new(Marker {
        buffer,
        anchor: Some(anchor),
        bias: Bias::Right, last_position: 0
    })));
    let overlay = vm.allocate_object(Object::Overlay(Box::new(Overlay {
        buffer: Value::NIL,
        start: None,
        end: None,
        front: Bias::Left,
        rear: Bias::Right,
        properties: Vec::new(),
        inserted: 0,
    })));
    vm.move_overlay_object(
        overlay,
        Value::integer(2).unwrap(),
        Value::integer(4).unwrap(),
        buffer,
    )
    .unwrap();
    let string = vm.allocate_object(Object::String(Box::new(data::LispString::new("λ"))));
    if let Object::String(s) = vm.objects.entry_mut(string.index(0).unwrap() - 1) {
        s.properties
            .put(ByteOffset(0)..ByteOffset(2), "overlay", overlay);
    }
    let property = vm.allocate_object(Object::Vector(vec![marker, overlay, buffer, string]));
    let slot = vm.slot("annotation-test");
    if let Object::Overlay(o) = vm.objects.entry_mut(overlay.index(0).unwrap() - 1) {
        o.properties.push((Value::reference(slot + 1, 7), property));
    }
    let mut roots = [buffer];
    vm.collect(&mut roots);
    assert_ne!(roots[0], buffer, "dead prefix must force compaction");
    let b = vm.annotated(roots[0]).unwrap();
    let overlay = b.intervals.query(2, 2, true, &b.anchors).0[0];
    let b = vm.annotated(roots[0]).unwrap();
    assert_eq!(b.intervals.query(2, 2, true, &b.anchors).0, [overlay]);
    assert_eq!(
        vm.overlay_property_named(overlay, "annotation-test")
            .unwrap(),
        vm.overlay(overlay).unwrap().properties[0].1
    );
    let o = vm.overlay(overlay).unwrap();
    assert_eq!(o.buffer, roots[0]);
    let Object::Vector(v) = vm.object(o.properties[0].1).unwrap() else {
        panic!()
    };
    let marker = v[0];
    assert_eq!(v[1], overlay);
    assert_eq!(v[2], roots[0]);
    let Object::String(s) = vm.object(v[3]).unwrap() else {
        panic!()
    };
    assert_eq!(
        s.properties.get_property(ByteOffset(0), "overlay"),
        Some(overlay)
    );
    assert_eq!(vm.marker(marker).unwrap().buffer, roots[0]);
    vm.annotated_mut(roots[0])
        .unwrap()
        .replace(ByteOffset(1)..ByteOffset(1), "ξЮ".as_bytes())
        .unwrap();
    assert_eq!(
        vm.marker_position_value(marker)
            .unwrap()
            .as_integer()
            .unwrap(),
        4
    );
    assert_eq!(vm.overlay_range(overlay).unwrap(), Some((1, 9)));
    // Remove the buffer-owned overlay edge: the marker/property cycle becomes unreachable.
    vm.delete_overlay_object(overlay).unwrap();
    vm.collect(&mut roots);
    assert_eq!(vm.heap_objects(), 1);
    assert!(vm.annotated(roots[0]).unwrap().anchors.is_empty());
    let new = vm
        .annotated_mut(roots[0])
        .unwrap()
        .anchors
        .create(0, Bias::Left)
        .unwrap();
    assert_ne!(new, anchor);
    assert_eq!(
        vm.annotated(roots[0]).unwrap().anchors.resolve(anchor),
        None
    );
    vm.collect(&mut []);
    assert_eq!(vm.heap_objects(), 0);
}
#[test]
fn notmuch_buffer_queries_use_the_interval_index() {
    let mut vm = Vm::new();
    let buffer = vm.allocate_object(Object::Buffer(Box::new(
        text::Text::new(&vec![b'x'; 886 * 1024]).unwrap().into(),
    )));
    vm.slots[vm.buffer_slot].value = buffer;
    let mut overlays = vec![Value::NIL; 6673];
    for j in 0..6673 {
        let i = (j * 197) % 6673;
        let v = vm.allocate_object(Object::Overlay(Box::new(Overlay {
            buffer: Value::NIL,
            start: None,
            end: None,
            front: Bias::Left,
            rear: Bias::Right,
            properties: Vec::new(),
            inserted: 0,
        })));
        vm.move_overlay_object(
            v,
            Value::integer(i as i64 * 100 + 1).unwrap(),
            Value::integer(i as i64 * 100 + 51).unwrap(),
            buffer,
        )
        .unwrap();
        overlays[i] = v;
    }
    assert_eq!(vm.annotated(buffer).unwrap().intervals.len(), 6673);
    for i in [0, 1, 3336, 6672] {
        let at = Value::integer(i as i64 * 100 + 26).unwrap();
        vm.stack.push(at);
        let result = vm
            .annotation_primitive(Primitive::OverlaysAt, 0, 1)
            .unwrap();
        vm.stack.clear();
        assert_eq!(vm.list_values(result).unwrap(), [overlays[i]]);
        vm.stack
            .extend([at, Value::integer(i as i64 * 100 + 30).unwrap()]);
        let result = vm
            .annotation_primitive(Primitive::OverlaysIn, 0, 2)
            .unwrap();
        vm.stack.clear();
        assert_eq!(vm.list_values(result).unwrap(), [overlays[i]]);
        vm.stack.push(at);
        let next = vm
            .annotation_primitive(Primitive::NextOverlayChange, 0, 1)
            .unwrap();
        vm.stack.clear();
        assert_eq!(next.as_integer().unwrap(), i as i64 * 100 + 51);
    }
    let b = vm.annotated_mut(buffer).unwrap();
    b.replace(ByteOffset(4090)..ByteOffset(4120), "λЖEnglish".as_bytes())
        .unwrap();
    let mut before_append = b.len_bytes();
    b.point = ByteOffset(before_append);
    let mark = b.anchors.create(before_append as u32, Bias::Right).unwrap();
    for _ in 0..9276 {
        assert!(b.append_utf8("λЖ"));
        before_append += "λЖ".len();
        assert_eq!(b.anchors.resolve(mark), Some(before_append as u32));
    }
    assert_eq!(b.anchors.extent() as usize, b.len_bytes());
    let mut called = 0;
    vm.visit_overlays_at(buffer, ByteOffset(1025), |value, range| {
        assert_eq!(value, overlays[10]);
        assert_eq!(range, ByteOffset(1000)..ByteOffset(1050));
        called += 1;
    })
    .unwrap();
    assert_eq!(called, 1);
    called = 0;
    assert!(
        vm.visit_overlays_at(buffer, ByteOffset(4091), |_, _| called += 1)
            .is_err()
    );
    assert!(
        vm.visit_overlays_in(buffer, ByteOffset(2)..ByteOffset(1), |_, _| called += 1)
            .is_err()
    );
    assert!(
        vm.visit_overlays_at(Value::NIL, ByteOffset(0), |_, _| called += 1)
            .is_err()
    );
    assert_eq!(called, 0);
}
#[test]
fn hermes_shape_reuses_three_overlay_slots_for_9276_moves() {
    let mut vm = Vm::new();
    let buffer = vm.allocate_object(Object::Buffer(Box::new(
        text::Text::new(&vec![b'x'; 8192]).unwrap().into(),
    )));
    let mut overlays = Vec::new();
    for _ in 0..3 {
        let v = vm.allocate_object(Object::Overlay(Box::new(Overlay {
            buffer: Value::NIL,
            start: None,
            end: None,
            front: Bias::Left,
            rear: Bias::Right,
            properties: Vec::new(),
            inserted: 0,
        })));
        vm.move_overlay_object(
            v,
            Value::integer(1).unwrap(),
            Value::integer(2).unwrap(),
            buffer,
        )
        .unwrap();
        overlays.push(v);
    }
    let before = vm.annotated(buffer).unwrap().intervals.storage_bytes(true);
    for i in 0..9276 {
        let v = overlays[i % 3];
        let a = (i * 197) % 8000;
        vm.move_overlay_object(
            v,
            Value::integer(a as i64 + 1).unwrap(),
            Value::integer(a as i64 + 8).unwrap(),
            buffer,
        )
        .unwrap();
        let b = vm.annotated(buffer).unwrap();
        let (found, visited) = b.intervals.query(a as u32, a as u32, true, &b.anchors);
        assert!(found.contains(&v));
        assert!(visited <= 3);
        assert_eq!(b.anchors.len(), 6);
        assert_eq!(b.intervals.len(), 3);
        assert_eq!(
            b.intervals.storage_bytes(true),
            before,
            "move {i} retained growth"
        );
    }
}
#[test]
fn identical_overlay_property_ties_keep_the_newest_identity_after_compaction() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    let buffer = vm.allocate_object(Object::Buffer(Box::new(
        text::Text::new(b"abc").unwrap().into(),
    )));
    vm.slots[vm.buffer_slot].value = buffer;
    let key = Value::reference(vm.slot("face") + 1, 7);
    for i in 1..=32 {
        vm.allocate_object(Object::String(Box::new(data::LispString::new(
            "dead prefix",
        ))));
        let overlay = vm.allocate_object(Object::Overlay(Box::new(Overlay {
            buffer: Value::NIL,
            start: None,
            end: None,
            front: Bias::Left,
            rear: Bias::Right,
            properties: vec![(key, Value::integer(i).unwrap())],
            inserted: 0,
        })));
        let buffer = vm.slots[vm.buffer_slot].value;
        vm.move_overlay_object(
            overlay,
            Value::integer(1).unwrap(),
            Value::integer(4).unwrap(),
            buffer,
        )
        .unwrap();
        vm.stack.extend([Value::integer(2).unwrap(), key]);
        let winner = vm
            .annotation_primitive(Primitive::GetCharProperty, 0, 2)
            .unwrap();
        vm.stack.clear();
        assert_eq!(
            winner.as_integer().unwrap(),
            i,
            "newest overlay must win at population {i}"
        );
        vm.collect(&mut []);
        let b = vm.annotated(vm.slots[vm.buffer_slot].value).unwrap();
        let newest = b
            .intervals
            .query(1, 1, true, &b.anchors)
            .0
            .into_iter()
            .max_by_key(|v| v.index(0).unwrap())
            .unwrap();
        assert_ne!(newest, overlay);
        vm.stack.extend([Value::integer(2).unwrap(), key]);
        let winner = vm
            .annotation_primitive(Primitive::GetCharProperty, 0, 2)
            .unwrap();
        vm.stack.clear();
        assert_eq!(winner.as_integer().unwrap(), i, "newest overlay after GC");
    }
}
#[test]
fn bulk_overlay_removal_scaling_and_evaporation_roots() {
    for evaporating in [false, true] {
        for population in [6673, 13346, 26692] {
            let cpu_ns = || {
                std::fs::read_to_string("/proc/thread-self/schedstat")
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
            };
            let mut samples = Vec::new();
            let mut cpu_samples = Vec::new();
            for _ in 0..3 {
                let mut vm = Vm::new();
                vm.allocate_object(Object::String(Box::new(data::LispString::new("dead"))));
                let buffer = vm.allocate_object(Object::Buffer(Box::new(
                    text::Text::new(&vec![b'x'; population * 2]).unwrap().into(),
                )));
                vm.slots[vm.buffer_slot].value = buffer;
                for i in 0..population {
                    let overlay = vm.allocate_object(Object::Overlay(Box::new(Overlay {
                        buffer: Value::NIL,
                        start: None,
                        end: None,
                        front: Bias::Left,
                        rear: Bias::Left,
                        properties: Vec::new(),
                        inserted: 0,
                    })));
                    vm.move_overlay_object(
                        overlay,
                        Value::integer((i * 2 + 1) as i64).unwrap(),
                        Value::integer((i * 2 + 2) as i64).unwrap(),
                        buffer,
                    )
                    .unwrap();
                    if evaporating {
                        let anchor = vm.overlay(overlay).unwrap().start.unwrap();
                        vm.annotated_mut(buffer)
                            .unwrap()
                            .evaporating
                            .insert(anchor, overlay);
                    }
                }
                vm.collect(&mut []);
                let buffer = vm.slots[vm.buffer_slot].value;
                assert_eq!(vm.annotated(buffer).unwrap().intervals.len(), population);
                for (&anchor, &overlay) in &vm.annotated(buffer).unwrap().evaporating {
                    assert_eq!(vm.overlay(overlay).unwrap().start, Some(anchor));
                    assert_eq!(vm.overlay(overlay).unwrap().buffer, buffer);
                }
                let cpu_start = cpu_ns();
                let start = std::time::Instant::now();
                vm.annotation_primitive(Primitive::RemoveOverlays, 0, 0)
                    .unwrap();
                samples.push(start.elapsed().as_nanos());
                cpu_samples.push(cpu_ns() - cpu_start);
                let b = vm.annotated(buffer).unwrap();
                assert_eq!(b.intervals.len(), 0);
                assert!(b.anchors.is_empty());
                assert!(b.evaporating.is_empty());
                vm.collect(&mut []);
                assert_eq!(vm.heap_objects(), 1);
            }
            samples.sort_unstable();
            cpu_samples.sort_unstable();
            eprintln!(
                "bulk-remove population={population} evaporating={evaporating} median_ns={} median_cpu_ns={} samples_ns={samples:?} cpu_samples_ns={cpu_samples:?}",
                samples[1], cpu_samples[1]
            );
        }
    }
}
#[test]
fn overlay_identity_is_not_a_copyable_sequence() {
    let mut vm = Vm::new();
    let f=vm.compile(&Form::from_json(serde_json::json!(["with-temp-buffer",["insert",{"string":"abc"}],["copy-sequence",["make-overlay",1,3]]])).unwrap()).unwrap();
    assert!(vm.execute_raw(f).unwrap_err().contains("sequence required"));
}

#[test]
fn append_tracks_extent_across_empty_and_registered_anchor_states() {
    let mut buffer: annotations::AnnotatedBuffer = text::Text::new(b"").unwrap().into();
    for _ in 0..300 {
        assert!(buffer.append_utf8("λ🙂"));
        assert_eq!(buffer.anchors.extent() as usize, buffer.len_bytes());
    }
    let end = buffer.anchors.extent();
    let left = buffer.anchors.create(end, Bias::Left).unwrap();
    let right = buffer.anchors.create(end, Bias::Right).unwrap();
    assert!(buffer.append_utf8("abc"));
    assert_eq!(buffer.anchors.resolve(left), Some(end));
    assert_eq!(buffer.anchors.resolve(right), Some(end + 3));
    assert!(buffer.anchors.remove(left));
    assert!(buffer.anchors.remove(right));
    assert!(buffer.append_utf8("Ж"));
    assert_eq!(buffer.anchors.extent() as usize, buffer.len_bytes());
    let extent = buffer.anchors.extent();
    let last = buffer.anchors.create(extent, Bias::Right).unwrap();
    assert!(buffer.append_utf8("!"));
    assert_eq!(
        buffer.anchors.resolve(last),
        Some(buffer.len_bytes() as u32)
    );
}

#[test]
fn append_reuses_scalar_count_across_tail_flushes() {
    let mut buffer: annotations::AnnotatedBuffer = text::Text::new(b"").unwrap().into();
    let chunk = "🙂".repeat(128);
    for n in 1..=9 {
        assert!(buffer.append_utf8(&chunk));
        assert_eq!(buffer.len_chars(), n * 128);
        assert_eq!(buffer.len_bytes(), n * 512);
        assert_eq!(buffer.anchors.extent() as usize, n * 512);
        assert_eq!(
            buffer.byte_of_char(CharOffset(n * 128)).unwrap(),
            ByteOffset(n * 512)
        );
    }
    assert!(!buffer.append_utf8(&"🙂".repeat(129)));
    assert!(!buffer.append_utf8(&"x".repeat(129)));
    assert_eq!(buffer.len_chars(), 9 * 128);
    assert_eq!(buffer.anchors.extent(), 9 * 512);
    assert!(buffer.append_utf8(&"x".repeat(128)));
    assert_eq!(buffer.len_chars(), 10 * 128);
    assert_eq!(buffer.len_bytes(), 9 * 512 + 128);
}

#[test]
fn refused_append_preserves_text_and_anchor_extent() {
    let mut buffer: annotations::AnnotatedBuffer = text::Text::new(b"abc").unwrap().into();
    buffer.point = ByteOffset(0);
    assert!(!buffer.append_utf8("x"));
    buffer.point = ByteOffset(3);
    assert!(!buffer.append_utf8(&"x".repeat(513)));
    assert!(!buffer.append_utf8(&"λ".repeat(129)));
    assert_eq!(buffer.len_bytes(), 3);
    assert_eq!(buffer.anchors.extent(), 3);
    let end = buffer.anchors.create(3, Bias::Right).unwrap();
    assert!(!buffer.append_utf8(&"x".repeat(513)));
    assert_eq!(buffer.anchors.resolve(end), Some(3));
    assert!(buffer.append_utf8("!"));
    assert_eq!(buffer.anchors.resolve(end), Some(4));
}

/// `text-property-any`, `text-property-not-all` and
/// `next-single-property-change` cost the runs of the properties that
/// decide the one asked about, not the characters nor every other
/// property's runs: font-lock asks them over whole comments on each key.
#[test]
fn single_property_scans_cost_the_deciding_runs() {
    let mut vm = Vm::new();
    // 20,000 characters, 2,000 runs of `face', three stretches of `mark'.
    vm.eval_source_raw(
        "(progn (set-buffer (get-buffer-create \"scan\")) (insert (make-string 20000 ?x)) \
           (dotimes (i 2000) (put-text-property (+ 1 (* i 10)) (+ 6 (* i 10)) 'face 'bold)) \
           (put-text-property 5000 5010 'mark t) (put-text-property 9000 9001 'mark 'other) \
           (put-text-property 15000 15003 'mark t))",
    ).unwrap();
    let probes = |vm: &mut Vm, source: &str, expected: &str| {
        vm.variable_read_probes.set(0);
        let value = vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.display(value).unwrap(), expected, "{source}");
        vm.variable_read_probes.get()
    };
    for (source, expected) in [
        ("(text-property-any 1 20001 'mark 'absent)", "nil"),
        ("(text-property-any 1 20001 'mark 'other)", "9000"),
        ("(text-property-any 5003 20001 'mark t)", "5003"),
        ("(text-property-not-all 5000 20001 'mark t)", "5010"),
        ("(text-property-not-all 1 4000 'mark nil)", "nil"),
        ("(next-single-property-change 1 'mark)", "5000"),
        ("(next-single-property-change 5010 'mark nil 9000)", "9000"),
        ("(next-single-property-change 15003 'mark)", "nil"),
        ("(next-single-property-change 15003 'absent nil 20001)", "20001"),
    ] {
        let count = probes(&mut vm, source, expected);
        assert!(count <= 40, "{source}: {count} symbol reads for at most seven runs of `mark'");
    }
}
