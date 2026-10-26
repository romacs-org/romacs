fn int(n: i64) -> Value { Value::integer(n).unwrap() }
impl Leaf<'_> {
    fn charset_pair(&self, value: Value) -> std::result::Result<(Value, Value), Fault> {
        if value == Value::NIL { return Ok((Value::NIL, Value::NIL)); }
        self.pair(value).ok_or(Fault::Wrong("listp", value))
    }
    fn charset_map_entries(&mut self, map: Value, min: u32, max: u32) -> std::result::Result<Vec<crate::charset::map::Entry>, Fault> {
        if matches!(self.kind(map), Kind::String | Kind::Bytes) {
            use std::os::unix::ffi::OsStringExt;
            let filename = std::ffi::OsString::from_vec(self.string_view(map)?.bytes().to_vec());
            let slot = self.vm.slot("charset-map-path");
            self.vm.specials.insert("charset-map-path".into());
            if self.vm.slots[slot].value == Value::UNBOUND {
                let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../etc/charsets");
                let path = self.string(&directory.to_string_lossy());
                self.vm.slots[slot].value = self.cons(path, Value::NIL);
            }
            let mut directories = Vec::new();
            let mut tail = self.vm.slots[slot].value;
            while let Some((head, rest)) = self.pair(tail) {
                if head == Value::NIL { directories.push(std::path::PathBuf::new()); }
                else { directories.push(std::path::PathBuf::from(std::ffi::OsString::from_vec(self.string_view(head)?.bytes().to_vec()))); }
                tail = rest;
            }
            if std::path::Path::new(&filename).is_absolute() { directories = vec![std::path::PathBuf::new()]; }
            let default_symbol = self.intern("default-directory");
            let default = self.variable_value(default_symbol)?.unwrap_or(Value::NIL);
            let base = if matches!(self.kind(default), Kind::String | Kind::Bytes) {
                std::path::PathBuf::from(std::ffi::OsString::from_vec(self.string_view(default)?.bytes().to_vec()))
            } else { std::env::current_dir().map_err(|e| Fault::Error(e.to_string()))? };
            for directory in directories {
                let directory = if directory.is_absolute() { directory } else { base.join(directory) };
                for suffix in [".map", ".txt"] {
                    let mut suffixed = filename.clone();
                    suffixed.push(suffix);
                    let path = directory.join(suffixed);
                    match std::fs::read(path) {
                        Ok(bytes) => return Ok(crate::charset::map::read_entries(&bytes, min, max)),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                        Err(error) => return Err(Fault::Error(format!("Loading charset map: {error}"))),
                    }
                }
            }
            let condition = self.intern("file-missing");
            let operation = self.string("Loading charset map");
            let message = self.string("No such file or directory");
            let data = self.cons(map, Value::NIL);
            let data = self.cons(message, data);
            let data = self.cons(operation, data);
            return Err(Fault::Signal(condition, data));
        }
        let len = match self.vm.object_if(map) {
            Some(data::Object::Vector(values)) => values.len(),
            _ => return Err(Fault::Wrong("vectorp", map)),
        };
        let mut entries = Vec::new();
        // The pinned GNU logger rejects its %V format before publication.
        if len % 2 != 0 { return Err(Fault::Error("Invalid format operation %V".into())); }
        for i in (0..len).step_by(2) {
            let code = self.charset_aref(map, i)?;
            let (from, to) = self.pair(code).unwrap_or((code, code));
            let from = from.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", from))? as u32;
            let to = to.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", to))? as u32;
            let character = self.charset_aref(map, i+1)?;
            let c = character.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", character))?;
            if from >= min && to <= max && from <= to && c <= 0x3fffff {
                entries.push(crate::charset::map::Entry { from, to, character: c as u32 });
            }
        }
        Ok(entries)
    }
    fn charset_range(&self, v: Value, lo: i64, hi: i64) -> std::result::Result<i64, Fault> {
        if self.kind(v) != Kind::Integer && self.kind(v) != Kind::Bignum { return Err(Fault::Wrong("integerp", v)); }
        let n = v.integer_value().filter(|n| *n >= lo && *n <= hi)
            .ok_or(Fault::Range3(v, int(lo), int(hi)))?;
        Ok(n)
    }
    fn charset_unsigned(&self, v: Value) -> std::result::Result<u32, Fault> {
        let value = if let Some((hi, tail)) = self.pair(v) {
            let hi = hi.integer_value().and_then(|n| u64::try_from(n).ok());
            let low = if let Some((mid, lo)) = self.pair(tail) {
                match (mid.integer_value(), lo.integer_value()) {
                    (Some(mid), Some(lo)) if (0..1<<24).contains(&mid) && (0..65536).contains(&lo) => hi.filter(|h| *h <= u64::MAX >> 40).map(|h| h << 40).map(|h| h | (mid as u64)<<16 | lo as u64).or_else(|| hi.filter(|_| mid < 65536).and_then(|h| h.checked_mul(65536).map(|h| h | mid as u64))),
                    _ => hi.and_then(|h| mid.integer_value().filter(|n| (0..65536).contains(n)).and_then(|l| h.checked_mul(65536).map(|h| h | l as u64))),
                }
            } else { hi.and_then(|h| tail.integer_value().filter(|n| (0..65536).contains(n)).and_then(|l| h.checked_mul(65536).map(|h| h | l as u64))) };
            low
        } else if let Some(n) = v.integer_value() { u64::try_from(n).ok() }
        else if let Some(data::Object::Float(n)) = self.vm.object_if(v) {
            if *n >= 0.0 && *n <= u32::MAX as f64 && n.fract() == 0.0 { Some(*n as u64) } else { None }
        } else { None };
        value.and_then(|n| u32::try_from(n).ok()).ok_or(Fault::Error("Not an in-range integer, integral float, or cons of integers".into()))
    }
    fn charset_aref(&mut self, v: Value, i: usize) -> super::super::Out {
        match self.kind(v) {
            Kind::Vector | Kind::Record => match self.vm.object_if(v) {
                Some(data::Object::Vector(values) | data::Object::Record(values)) => values.get(i).copied().ok_or(Fault::Range(v, int(i as i64))),
                _ => unreachable!(),
            },
            Kind::CharTable => self.char_table_element(v, i),
            Kind::String | Kind::Bytes => self.string_view(v)?.chars().nth(i).map(|c| int(c as i64)).ok_or(Fault::Range(v, int(i as i64))),
            _ => Err(Fault::Wrong("arrayp", v)),
        }
    }
    pub fn charset_define(&mut self, a: &[Value]) -> super::super::Out {
        if self.kind(a[0]) != Kind::Symbol { return Err(Fault::Wrong("symbolp", a[0])); }
        let mut space = [0u32; 8];
        let mut dimension = 0;
        for i in 0..4 {
            let lo = self.charset_aref(a[2], 2*i)?;
            let hi = self.charset_aref(a[2], 2*i+1)?;
            space[2*i] = self.charset_range(lo, 0, 255)? as u32;
            space[2*i+1] = self.charset_range(hi, space[2*i] as i64, 255)? as u32;
            if space[2*i+1] > 0 { dimension = i as i64+1; }
        }
        if a[1] != Value::NIL { dimension = self.charset_range(a[1], 1, 4)?; }
        let linear = dimension == 1 || (space[1]-space[0] == 255 && (dimension == 2 || (space[3]-space[2] == 255 && (dimension == 3 || space[5]-space[4] == 255))));
        let pack = |odd: usize| -> u32 { (0..4).fold(0, |v,i| v | space[2*i+odd] << (8*i)) };
        let mut min = pack(0);
        let mut max = pack(1);
        let index = |code: u32, min: u32, index_offset: i32| -> i32 {
            if linear { return code.wrapping_sub(min) as i32; }
            let mut idx = 0u32;
            let mut stride = 1u32;
            for i in 0..4 {
                let byte = (code >> (8*i)) & 255;
                if byte < space[2*i] || byte > space[2*i+1] { return -1; }
                idx = idx.wrapping_add((byte-space[2*i]).wrapping_mul(stride));
                stride = stride.wrapping_mul(space[2*i+1]-space[2*i]+1);
            }
            idx.wrapping_sub(index_offset as u32) as i32
        };
        let mut index_offset = 0;
        if a[3] != Value::NIL {
            let code = self.charset_unsigned(a[3])?;
            if code < min || code > max { return Err(Fault::Range3(int(min as i64), int(max as i64), a[3])); }
            index_offset = index(code, min, 0);
            min = code;
        }
        if a[4] != Value::NIL {
            let code = self.charset_unsigned(a[4])?;
            if code < min || code > max { return Err(Fault::Range3(int(min as i64), int(max as i64), a[4])); }
            max = code;
        }
        let invalid = if a[10] != Value::NIL { self.charset_unsigned(a[10])? } else if min > 0 { 0 } else { max.checked_add(1).ok_or(Fault::Error("Attribute :invalid-code must be specified".into()))? };
        let iso_final = if a[5] == Value::NIL { -1 } else {
            let n = a[5].integer_value().ok_or(Fault::Wrong("fixnump", a[5]))?;
            if !(48..=127).contains(&n) { return Err(Fault::Error(format!("Invalid iso-final-char: {n}"))); } n
        };
        let iso_revision = if a[6] == Value::NIL { -1 } else { self.charset_range(a[6], -1, 63)? };
        let mule_id = if a[7] == Value::NIL { -1 } else {
            let n = a[7].integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", a[7]))?;
            if (n > 0 && n <= 128) || n >= 256 { return Err(Fault::Error(format!("Invalid emacs-mule-id: {n}"))); } n
        };
        let offset = if a[11] != Value::NIL { Some(a[11].integer_value().filter(|n| (0..=0x3fffff).contains(n)).ok_or(Fault::Wrong("characterp", a[11]))?) } else { None };
        let mut subset = Value::NIL;
        let parent_metadata = if offset.is_none() && a[12] == Value::NIL && a[13] != Value::NIL {
            let parent = self.charset_pair(a[13])?.0;
            let parent_index = self.charset_record(parent)?;
            let parent_id = self.vm.charsets.records[parent_index].attrs[0];
            let mut tail = self.charset_pair(a[13])?.1;
            let parent_min = self.charset_pair(tail)?.0;
            if parent_min.integer_value().is_none_or(|n| n < 0) { return Err(Fault::Wrong("wholenump", parent_min)); }
            tail = self.charset_pair(tail)?.1;
            let parent_max = self.charset_pair(tail)?.0;
            if parent_max.integer_value().is_none_or(|n| n < 0) { return Err(Fault::Wrong("wholenump", parent_max)); }
            tail = self.charset_pair(tail)?.1;
            let parent_offset = self.charset_pair(tail)?.0;
            if parent_offset.integer_value().is_none() { return Err(Fault::Wrong("fixnump", parent_offset)); }
            subset = self.vector(&[parent_id, parent_min, parent_max, parent_offset]);
            let parent = self.vm.charsets.native_record(parent).expect("validated parent");
            Some((parent.min_char, parent.max_char, parent.fast_map))
        } else { None };
        let mut superset = Value::NIL;
        let parent_metadata = if offset.is_none() && a[12] == Value::NIL && parent_metadata.is_none() && a[14] != Value::NIL {
            // GNU copies the whole sequence before validating any parent.
            if self.pair(a[14]).is_none() {
                if matches!(self.kind(a[14]), Kind::String | Kind::Bytes | Kind::Vector | Kind::Record | Kind::CharTable | Kind::SyntaxTable) {
                    let copy = self.vm.object(a[14]).map_err(|_| Fault::Wrong("sequencep", a[14]))?.clone();
                    let copy = self.vm.allocate_object(copy);
                    return Err(Fault::Wrong("listp", copy));
                }
                return Err(Fault::Wrong("sequencep", a[14]));
            }
            let mut tail = a[14];
            let mut seen = HashSet::new();
            while tail != Value::NIL {
                if !seen.insert(tail.0) { return Err(Fault::Circular(a[14])); }
                tail = self.charset_pair(tail)?.1;
            }
            let mut tail = a[14];
            let mut parents = Vec::new();
            let mut min_char = 0x3fffff;
            let mut max_char = 0;
            let mut fast_map = [0u8; 190];
            while tail != Value::NIL {
                let (entry, rest) = self.charset_pair(tail)?;
                let (name, offset) = self.pair(entry).unwrap_or((entry, int(0)));
                let index = self.charset_record(name)?;
                let offset = self.charset_range(offset, i32::MIN as i64, i32::MAX as i64)?;
                let parent_id = self.vm.charsets.records[index].attrs[0];
                let parent = self.vm.charsets.native_record(name).expect("validated parent");
                min_char = min_char.min(parent.min_char);
                max_char = max_char.max(parent.max_char);
                for (out, input) in fast_map.iter_mut().zip(parent.fast_map) { *out |= input; }
                parents.push(self.cons(parent_id, int(offset)));
                tail = rest;
            }
            for parent in parents.into_iter().rev() { superset = self.cons(parent, superset); }
            Some((min_char, max_char, fast_map))
        } else { parent_metadata };
        if offset.is_none() && a[12] == Value::NIL && parent_metadata.is_none() {
            return Err(Fault::Error("None of :code-offset, :map, :parents are specified".into()));
        }
        let high = index(max, min, index_offset);
        let (min_char, max_char, fast_map) = if let Some(offset) = offset {
            if 0x3fffff-offset < high as i64 { return Err(Fault::Error(format!("Unsupported max char: {high} + {max}d > MAX_CHAR (4194303)"))); }
            let max_char = high as i64 + offset;
            let min_char = index(min, min, index_offset) as i64 + offset;
            (min_char, max_char, crate::charset::offset_fast_map(min_char, max_char))
        } else { parent_metadata.unwrap_or((0, 0, [0; 190])) };
        if a[15] != Value::NIL && !matches!(self.kind(a[15]), Kind::String | Kind::Bytes | Kind::Vector) { return Err(Fault::Wrong("vectorp", a[15])); }
        if a[16] != Value::NIL && self.pair(a[16]).is_none() { return Err(Fault::Wrong("listp", a[16])); }
        self.vm.ensure_charsets();
        let old = self.vm.charsets.lookup(a[0]);
        let id = old.map(|i| self.vm.charsets.records[i].attrs[0].integer_value().unwrap() as usize).unwrap_or(self.vm.charsets.ids.len());
        let mut attrs = [Value::NIL; 10]; attrs[0]=int(id as i64); attrs[1]=a[0]; attrs[2]=a[16]; attrs[8]=a[15];
        if offset.is_none() { attrs[3] = a[12]; attrs[6] = subset; attrs[7] = superset; }
        let record = crate::charset::Record { attrs, space, dimension, min, max, offset: offset.map(|n| n as u32), iso_final, iso_revision, mule_id, ascii_compatible: a[8] != Value::NIL || (offset == Some(0) && max_char >= 128), supplementary: a[9] != Value::NIL, invalid, min_char, max_char, fast_map, linear, index_offset, unified: false, tables: None };
        // Attribute identity follows bindings, while native metadata follows ID.
        // Reuse an unshared replaced slot, or an unreachable retired slot, so
        // repeated redefinition does not retain obsolete Lisp plists forever.
        let reusable = old.filter(|old| self.vm.charsets.bindings.iter().filter(|(_,i)| i == old).count() == 1)
            .or_else(|| (0..self.vm.charsets.records.len()).find(|i| !self.vm.charsets.ids.contains(i) && !self.vm.charsets.bindings.iter().any(|(_,j)| j == i)));
        let i = reusable.unwrap_or(self.vm.charsets.records.len());
        if reusable.is_some() { self.vm.charsets.records[i] = record; }
        else { self.vm.charsets.records.push(record); }
        if let Some(binding) = self.vm.charsets.bindings.iter_mut().find(|(name,_)| *name == a[0]) { binding.1=i; } else { self.vm.charsets.bindings.push((a[0],i)); }
        if old.is_none() { self.vm.charsets.ids.push(i); } else { self.vm.charsets.ids[id]=i; }
        // GNU publishes name/ID before opening MAP, but publishes ordering,
        // ISO and Mule lists only after the initial read succeeds.
        if offset.is_none() && a[12] != Value::NIL {
            let entries = self.charset_map_entries(a[12], min, max)?;
            self.vm.charsets.records[i].initialize_map(&entries);
        }
        if iso_final >= 0 {
            let key=(dimension, space[1]-space[0]+1 == 96, iso_final);
            self.vm.charsets.publish_iso(key, id);
            if old.is_none() { self.vm.charsets.iso_list.push(id); }
        }
        if mule_id >= 0 {
            let bytes=dimension+if mule_id<160 {1} else {2};
            if let Some(entry)=self.vm.charsets.mule.iter_mut().find(|(k,_,_)| *k==mule_id) { *entry=(mule_id,id,bytes); } else { self.vm.charsets.mule.push((mule_id,id,bytes)); }
            if old.is_none() { self.vm.charsets.mule_list.push(id); }
        }
        if old.is_none() {
            let slot=self.vm.slot("charset-list");
            self.vm.slots[slot].value=self.cons(a[0],self.vm.slots[slot].value);
            let position=if a[9] != Value::NIL {self.vm.charsets.ordered.len()} else {self.vm.charsets.ordered.iter().position(|id| self.vm.charsets.records[self.vm.charsets.ids[*id]].supplementary).unwrap_or(self.vm.charsets.ordered.len())};
            self.vm.charsets.ordered.insert(position,id); self.vm.charsets.ordered_tick+=1;
        }
        Ok(Value::NIL)
    }
}
