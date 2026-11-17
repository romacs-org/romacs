//! The restored world image as a generation no collection frees. A restore
//! makes every entry of the image permanent or remembered (`table.rs`); to a
//! full's mark a permanent entry counts as reached, as an old one does to a
//! minor's, so a full reads of the image only the remembered entries and what
//! else may name an entry made since. Image entries are never freed and never
//! moved, as GNU's sweep frees no object of its dump (alloc.c
//! `pdumper_object_p`): what of the image becomes garbage is kept, at most
//! the image.
use crate::gc::{Census, shares_storage};
use crate::{Value, Vm, data, gc_mark, hash};
use std::sync::atomic::AtomicBool;

/// The restored world image (`Vm::make_image_permanent`), as a restore found
/// it: empty in a process that restored none.
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct WorldImage {
    /// Symbols of obarrays of their own then, by slot, in order.
    pub(crate) obarray_symbols: Vec<usize>,
    /// The census of its permanent objects, which a full counts without
    /// reading them (`garbage-collect`'s statistics).
    pub(crate) census: Census,
    /// Its objects remembered when `census` was taken: more since means
    /// some were written, and are counted by the mark that reads them.
    pub(crate) remembered: usize,
    /// Its entries per table: conses, closures, cells, objects.
    pub(crate) entries: [usize; 4],
    /// The epoch its storage was last written before (`crate::epoch`):
    /// char-table nodes stamped below it are as the image held them.
    pub(crate) since: u64,
    /// Its objects that hold storage written past their entries
    /// (`shares_storage`), by index, in order: never moved.
    pub(crate) holders: Vec<u32>,
}

impl WorldImage {
    /// Whether object I is one of the image's holders of storage.
    pub(crate) fn holds(&self, i: usize) -> bool {
        u32::try_from(i).is_ok_and(|i| self.holders.binary_search(&i).is_ok())
    }
}

impl Vm {
    /// The image's roots for a mark that is not a minor's, to which its
    /// permanent entries count as reached (`Table::raw`), onto PENDING and
    /// CODE: every remembered entry, stores into permanent ones since the
    /// last collection first recorded; the lexical pair of each permanent
    /// cell, which only a cell's own visit reads; the image's symbols of
    /// obarrays of their own, which a permanent obarray or list may hold
    /// untraced. Of the image's functions, which a permanent closure names
    /// untraced, it reads those changed since the restore; every other
    /// starts reached in the code marks, as an old one does to a minor, and of
    /// it only the lowered code made since is read.
    /// Whether there is an image: a process that restored none, as a
    /// builder, has nothing to read.
    pub(crate) fn image_roots(&mut self, pending: &mut Vec<Value>, code: &mut Vec<usize>, code_marks: &mut [AtomicBool]) -> bool {
        if self.world_image.entries == [0; 4] {
            return false;
        }
        let [conses, closures, cells, objects] = [self.conses.remembered(), self.closures.remembered(), self.cells.remembered(), self.objects.remembered()];
        self.gc_times.remembered = [conses.len(), closures.len(), cells.len(), objects.len()];
        // Remembered entries are never permanent again: as many as when the
        // census was taken means none was written since.
        if objects.len() != self.world_image.remembered {
            self.world_image.census = self.permanent_census();
            self.world_image.remembered = objects.len();
        }
        pending.extend(conses.into_iter().map(|i| Value::reference(i, 3)));
        pending.extend(closures.into_iter().map(|i| Value::reference(i, 4)));
        pending.extend(cells.into_iter().map(|i| Value::reference(i, 5)));
        pending.extend(objects.into_iter().map(|i| Value::reference(i + 1, 0)));
        pending.extend(self.cell_pairs.iter().filter(|&(&i, _)| self.cells.permanent(i)).map(|(_, &pair)| pair));
        let symbols = &self.obarray_symbol_names;
        pending.extend(self.world_image.obarray_symbols.iter().filter(|slot| symbols.contains_key(slot)).map(|&slot| Value::reference(slot + 1, 7)));
        for id in self.functions.image_untouched() {
            *code_marks[id].get_mut() = true;
            for program in std::iter::once(&self.execution).chain(&self.spare_programs) {
                program.trace_function(id, pending, code);
            }
        }
        code.extend_from_slice(self.functions.image_touched());
        true
    }
    /// The census of the image's permanent objects, as the mark would count them.
    fn permanent_census(&self) -> Census {
        let mut census = Census::default();
        for (i, object) in self.objects.iter().enumerate() {
            if self.objects.permanent(i) { census.add(&gc_mark::census_of(object)); }
        }
        census
    }
    /// What a restore makes of the world image it maps: a generation no
    /// collection frees or moves, of which a full reads only what may name
    /// the rest (`image_roots`). Buffers, char tables and the other holders
    /// of storage written past their entries, and weak hash tables, whose
    /// entries and buffer-local bindings only a mark's weak rounds keep, are
    /// remembered; every other entry is permanent.
    pub(crate) fn make_image_permanent(&mut self) {
        let remembered = |object: &data::Object| shares_storage(object) || matches!(object, data::Object::HashTable(t) if t.weakness != hash::Weakness::None);
        self.conses.make_permanent(|_| false);
        self.closures.make_permanent(|_| false);
        self.cells.make_permanent(|_| false);
        self.objects.make_permanent(remembered);
        let holders = self.objects.iter().enumerate().filter(|&(_, object)| shares_storage(object)).map(|(i, _)| u32::try_from(i).expect("an object's index fits a Value")).collect();
        self.functions.make_image(self.request_code.free_ids());
        let mut obarray_symbols: Vec<usize> = self.obarray_symbol_names.keys().copied().collect();
        obarray_symbols.sort_unstable();
        self.world_image = WorldImage {
            obarray_symbols,
            // Taken at the first full (`image_roots`).
            census: Census::default(),
            remembered: usize::MAX,
            entries: [self.conses.live(), self.closures.live(), self.cells.live(), self.objects.live()],
            since: self.allocation.since,
            holders,
        };
    }
}

#[cfg(test)]
mod tests {
    use crate::{Value, Vm};

    fn eval(vm: &mut Vm, source: &str) -> Value {
        vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e:?}"))
    }
    fn is_true(vm: &mut Vm, source: &str) -> bool {
        eval(vm, source) == Value::TRUE
    }

    /// A VM whose heap is made the image's, as a restore makes it, after
    /// SETUP and a collection; verify on, so every full's mark is held to
    /// the reference walk, which reads through permanent entries.
    fn imaged(setup: &str) -> Vm {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        eval(&mut vm, setup);
        vm.collect(&mut []);
        vm.make_image_permanent();
        vm
    }

    /// A full's mark leaves an untouched image entry unread, reads the ones
    /// written since, and reaches everything they were given: the young
    /// values stored into a cons, a vector, a strong and a weak hash table
    /// and a closure's cell of the image all survive fulls, and the image's
    /// closures and functions still run.
    #[test]
    fn a_full_reads_no_permanent_entry_and_keeps_what_the_image_was_given() {
        let mut vm = imaged("(setq img-list (list 1 2 3) img-vector (make-vector 4 nil) img-table (make-hash-table) \
            img-weak (make-hash-table :weakness 'key) img-untouched (list 'a 'b) \
            img-counter (let ((n nil)) (lambda () (setq n (cons (length n) n)))))");
        let untouched = eval(&mut vm, "img-untouched").index_if(3).unwrap();
        let list = eval(&mut vm, "img-list").index_if(3).unwrap();
        let table = eval(&mut vm, "img-table").index_if(0).unwrap() - 1;
        assert!(vm.conses.permanent(untouched) && vm.conses.permanent(list) && vm.objects.permanent(table));
        eval(&mut vm, "(progn (setcar img-list (list 4)) (aset img-vector 0 (list 5)) (puthash 'k (list 6) img-table) \
            (puthash img-list (list 7) img-weak) (funcall img-counter) (funcall img-counter) nil)");
        assert!(vm.objects.is_remembered(table), "a put into a permanent table is remembered at once");
        let young = eval(&mut vm, "(car img-list)").index_if(3).unwrap();
        vm.mark_only(&[], false);
        assert!(!vm.conses.claimed(untouched), "the full did not read the untouched cons");
        assert!(vm.conses.is_remembered(list) && vm.conses.claimed(list), "the written one is remembered and read");
        assert!(vm.conses.claimed(young), "and what it was given is reached");
        vm.clear_marks();
        eval(&mut vm, "(dotimes (_ 2000) (list 1 2 3))");
        for _ in 0..3 { vm.collect(&mut []); }
        assert!(is_true(&mut vm, "(equal (list (car img-list) (aref img-vector 0) (gethash 'k img-table) (gethash img-list img-weak) \
            img-untouched (funcall img-counter)) '((4) (5) (6) (7) (a b) (2 1 0)))"), "what the image was given survives");
    }

    /// Random young values written into image conses, vectors and hash
    /// tables survive minors, fulls and compactions interleaved: each slot
    /// prints at the end as it printed when written, under verify.
    #[test]
    fn young_values_written_into_the_image_survive_minors_fulls_and_compactions() {
        let mut vm = imaged("(progn (setq img-conses (make-vector 64 nil) img-vectors (make-vector 64 nil) img-tables (make-vector 16 nil)) \
            (dotimes (i 64) (aset img-conses i (cons nil nil)) (aset img-vectors i (make-vector 8 nil))) \
            (dotimes (i 16) (aset img-tables i (make-hash-table :test 'eql))))");
        // What each slot printed as when last written, outside the image.
        eval(&mut vm, "(progn (random \"imgold\") (setq img-printed (make-hash-table :test 'equal)))");
        let collections: [fn(&mut Vm); 3] = [|vm| vm.collect(&mut []), |vm| assert!(vm.collect_minor(&[])), Vm::collect_compacting];
        for round in 0..30 {
            eval(&mut vm, &format!("(let ((r {round})) (dotimes (k 40) \
                (let* ((i (random 64)) (j (random 8)) (v (list r k (make-string 3 ?x)))) \
                  (let ((kind (random 3))) \
                    (cond ((= kind 0) (setcar (aref img-conses i) v) (puthash (list 'c i) (prin1-to-string v) img-printed)) \
                          ((= kind 1) (aset (aref img-vectors i) j v) (puthash (list 'v i j) (prin1-to-string v) img-printed)) \
                          (t (let ((key (random 32)) (table (random 16))) (puthash key v (aref img-tables table)) \
                               (puthash (list 't table key) (prin1-to-string v) img-printed))))))) \
                (dotimes (_ 500) (list 1 2)))"));
            collections[round % 3](&mut vm);
        }
        let wrong = eval(&mut vm, "(let (wrong) (maphash (lambda (where printed) \
            (let ((now (cond ((eq (car where) 'c) (car (aref img-conses (nth 1 where)))) \
                             ((eq (car where) 'v) (aref (aref img-vectors (nth 1 where)) (nth 2 where))) \
                             (t (gethash (nth 2 where) (aref img-tables (nth 1 where))))))) \
              (unless (equal (prin1-to-string now) printed) (push where wrong)))) img-printed) wrong)");
        assert_eq!(wrong, Value::NIL, "every slot holds what was last written into it");
        assert!(is_true(&mut vm, "(> (hash-table-count img-printed) 100)"));
    }

    /// A compaction moves no image entry: an untouched image cons and a
    /// written one keep their places, and the young it was given move and
    /// are found again through it.
    #[test]
    fn a_compaction_moves_no_image_entry_and_rewrites_what_it_was_given() {
        let mut vm = imaged("(setq img-a (list 1 2 3) img-b (list 'b))");
        let (a, b) = (eval(&mut vm, "img-a").index_if(3).unwrap(), eval(&mut vm, "img-b").index_if(3).unwrap());
        eval(&mut vm, "(progn (dotimes (_ 3000) (list 1 2)) (setq img-young (list 9 9)) (dotimes (_ 3000) (list 1 2)) (setcar img-a img-young) nil)");
        vm.collect_compacting();
        assert_eq!((eval(&mut vm, "img-a").index_if(3), eval(&mut vm, "img-b").index_if(3)), (Some(a), Some(b)), "image entries stay");
        assert!(is_true(&mut vm, "(and (equal img-a '((9 9) 2 3)) (eq (car img-a) img-young) (equal img-b '(b)))"));
    }

    /// A buffer of the image keeps its buffer-local values, old and young,
    /// and so does a symbol of an obarray of the image's own, through fulls.
    #[test]
    fn image_buffers_locals_and_obarray_symbols_keep_their_values() {
        let mut vm = imaged("(progn (save-current-buffer (set-buffer (get-buffer-create \"img-buf\")) (set (make-local-variable 'img-local) (list 'old))) \
            (setq img-ob (obarray-make)) (set (intern \"img-sym\" img-ob) (list 'sym)) (setq img-holder (list (intern \"img-sym\" img-ob))))");
        eval(&mut vm, "(progn (save-current-buffer (set-buffer \"img-buf\") (set (make-local-variable 'img-new) (list 'new))) (dotimes (_ 2000) (list 1)) nil)");
        for _ in 0..2 { vm.collect(&mut []); }
        assert!(is_true(&mut vm, "(equal (list (buffer-local-value 'img-local (get-buffer \"img-buf\")) \
            (buffer-local-value 'img-new (get-buffer \"img-buf\")) (symbol-value (intern-soft \"img-sym\" img-ob)) (eq (car img-holder) (intern-soft \"img-sym\" img-ob))) \
            '((old) (new) (sym) t))"));
    }

    /// A permanent cell's lexical pair, named by `cell_pairs` alone, is a
    /// root of every full: one made since the restore survives with what it
    /// holds, and the cell that takes its value is remembered from then on.
    #[test]
    fn a_full_keeps_the_new_lexical_pair_of_a_permanent_cell() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        let cell = vm.alloc_cell(Value::TRUE);
        let mut roots = [Value::reference(cell, 5)];
        vm.collect(&mut roots);
        vm.make_image_permanent();
        assert!(vm.cells.permanent(cell));
        let young = vm.alloc_cons(Value::integer(5).unwrap(), Value::NIL);
        let pair = vm.alloc_cons(Value::NIL, young);
        vm.cell_pairs.insert(cell, pair);
        vm.collect(&mut roots);
        assert_eq!(vm.cells[cell], young, "the cell took its pair's value");
        assert!(vm.cells.is_remembered(cell), "and is read by every full from now on");
        vm.collect(&mut roots);
        assert_eq!(vm.conses[young.index_if(3).unwrap()].0, Value::integer(5).unwrap());
    }

    /// An image function changed in place since the restore is read by
    /// every full, and so is what it was given; an untouched one is not, but
    /// the lowered code made for it since is: a callee a call site of an
    /// image function entered, which nothing else names, is kept (verify's
    /// walk reads lowered code).
    #[test]
    fn a_full_reads_the_image_functions_changed_and_lowered_since_the_restore() {
        use crate::{Closure, Form, Op};
        let mut vm = imaged("(progn (defun img-caller () (img-callee)) (defun img-callee () 1) (img-caller))");
        let id = vm.compile(&Form::Int(0)).unwrap();
        let index = vm.closures.len();
        vm.closures.push(Closure { function: id, captures: vec![], metadata: vec![], document: Value::NIL });
        let mut roots = [Value::reference(index, 4)];
        vm.collect(&mut roots);
        vm.make_image_permanent();
        let young = vm.alloc_cons(Value::integer(7).unwrap(), Value::NIL);
        std::sync::Arc::get_mut(vm.functions.change(id)).unwrap().code = vec![Op::Const(0, young)];
        eval(&mut vm, "(progn (defun img-callee () (list 2)) (img-caller) (defun img-callee () 3) nil)");
        for _ in 0..2 { vm.collect(&mut roots); }
        assert_eq!(vm.conses[young.index_if(3).unwrap()].0, Value::integer(7).unwrap(), "the changed function's constant survived");
        assert!(is_true(&mut vm, "(equal (img-caller) 3)"));
    }

    /// An image char table's nodes written since the restore are read by a
    /// full, though the rest of it is not: what they were given survives.
    #[test]
    fn a_full_reads_the_image_char_table_nodes_written_since_the_restore() {
        let mut vm = imaged("(progn (setq img-chars (make-char-table 'img)) (aset img-chars ?a 0) (set-char-table-range img-chars '(#x4e00 . #x4e10) 0))");
        eval(&mut vm, "(progn (aset img-chars ?a (list 'a)) (set-char-table-range img-chars '(#x4e00 . #x4e10) (list 'range)) (aset img-chars ?z (list 'z)) (dotimes (_ 2000) (list 1)) nil)");
        for _ in 0..2 { vm.collect(&mut []); }
        assert!(is_true(&mut vm, "(equal (list (aref img-chars ?a) (aref img-chars #x4e08) (aref img-chars ?z)) '((a) (range) (z)))"));
    }

    /// Stores only a full depends on: a symbol interned since the restore
    /// into an image obarray of its own, which only a full reclaims, and a
    /// young value put as a text property on an image string, survive fulls,
    /// alone and after a minor.
    #[test]
    fn a_full_keeps_a_symbol_interned_and_a_property_put_into_the_image() {
        for minor_first in [false, true] {
            let mut vm = imaged("(setq img-ob (obarray-make) img-str (copy-sequence \"abc\"))");
            eval(&mut vm, "(progn (set (intern \"img-late\" img-ob) (list 'late)) (put-text-property 0 2 'img-prop (list 'young) img-str) \
                (dotimes (_ 2000) (list 1)) nil)");
            if minor_first { assert!(vm.collect_minor(&[])); }
            for _ in 0..2 { vm.collect(&mut []); }
            assert!(is_true(&mut vm, "(equal (list (symbol-value (intern-soft \"img-late\" img-ob)) (get-text-property 1 'img-prop img-str)) '((late) (young)))"),
                "minor first: {minor_first}");
        }
    }

    /// `garbage-collect`'s statistics count the image as before: a full of
    /// the same heap counts the same, whether its image is traced or not.
    #[test]
    fn the_census_counts_the_untraced_image() {
        let mut vm = Vm::new();
        eval(&mut vm, "(progn (setq img-strings nil img-vec (make-vector 100 nil) img-wide (string #x3b2 ?b)) (dotimes (i 500) (setq img-strings (cons (number-to-string i) img-strings))))");
        vm.collect(&mut []);
        vm.collect(&mut []);
        let traced = vm.gc_census;
        vm.make_image_permanent();
        vm.collect(&mut []);
        assert_eq!(vm.gc_census, traced, "an image counted as the mark counted it");
        eval(&mut vm, "(progn (aset img-vec 0 (make-vector 3 nil)) (aset img-wide 1 #x3b1) nil)");
        vm.collect(&mut []);
        assert_eq!((vm.gc_census.vectors, vm.gc_census.vector_slots), (traced.vectors + 1, traced.vector_slots + 3), "a written image vector counted once");
        assert_eq!(vm.gc_census.string_bytes, traced.string_bytes + 1, "a written image string counted as it is now");
    }

    /// A put into a permanent hash table remembers it whatever is put: a
    /// symbol of an obarray of its own, which only a full reclaims, survives
    /// a minor and then fulls with nothing else naming it.
    #[test]
    fn a_symbol_put_into_a_permanent_table_survives_fulls() {
        let mut vm = imaged("(setq img-table (make-hash-table))");
        eval(&mut vm, "(progn (puthash 'k (intern \"img-own\" (obarray-make)) img-table) nil)");
        assert!(vm.collect_minor(&[]));
        for _ in 0..2 { vm.collect(&mut []); }
        assert!(is_true(&mut vm, "(equal (symbol-name (gethash 'k img-table)) \"img-own\")"));
    }
}
