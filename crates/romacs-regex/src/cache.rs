use crate::{Regex, RegexError, Stats};
use std::{collections::HashMap, hash::BuildHasher, rc::Rc};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Literal,
    Regexp,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyRef<'a> {
    pub kind: Kind,
    pub pattern: &'a [u8],
    pub pattern_multibyte: bool,
    pub target_multibyte: bool,
    pub posix: bool,
    pub whitespace: Option<&'a [u8]>,
    pub case: Option<(u64, u64)>,
}
struct Key {
    kind: Kind,
    pattern: Box<[u8]>,
    pattern_multibyte: bool,
    target_multibyte: bool,
    posix: bool,
    whitespace: Option<Box<[u8]>>,
    case: Option<(u64, u64)>,
}
impl Key {
    fn as_ref(&self) -> KeyRef<'_> {
        KeyRef {
            kind: self.kind,
            pattern: &self.pattern,
            pattern_multibyte: self.pattern_multibyte,
            target_multibyte: self.target_multibyte,
            posix: self.posix,
            whitespace: self.whitespace.as_deref(),
            case: self.case,
        }
    }
}
impl From<KeyRef<'_>> for Key {
    fn from(key: KeyRef<'_>) -> Self {
        Self {
            kind: key.kind,
            pattern: key.pattern.into(),
            pattern_multibyte: key.pattern_multibyte,
            target_multibyte: key.target_multibyte,
            posix: key.posix,
            whitespace: key.whitespace.map(Into::into),
            case: key.case,
        }
    }
}
struct Entry {
    key: Key,
    regex: Rc<Regex>,
    used: u64,
}
/// Heap retained by a cache, independent of optional allocator statistics.
/// Programs include their inline `Regex`, `Rc` counters and owned arrays;
/// keys include copied pattern/whitespace bytes. Buckets include collision-vector
/// capacities and a conservative SwissTable storage charge (see `map_bytes`).
/// Excludes the inline `Cache` body, allocator metadata/rounding, temporary builds,
/// and programs kept alive only by callers after eviction. Shared programs are
/// charged once per retained entry, even when a caller also holds an `Rc`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheCensus {
    pub entries: usize,
    pub program_bytes: usize,
    pub key_bytes: usize,
    pub bucket_bytes: usize,
    pub retained_bytes: usize,
}

/// Owner-local cache: at most 128 entries and 256 KiB of charged retained heap.
/// Failed or oversized builds are not retained. All accounting/eviction work is
/// on misses or explicit census calls, never on the lookup hit path.
pub struct Cache {
    entries: HashMap<u64, Vec<Entry>>,
    len: usize,
    clock: u64,
}
impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}
impl Cache {
    pub const MAX_ENTRIES: usize = 128;
    // Enough for the 128-entry small-pattern working set without retaining
    // arbitrarily large literal/fold plans from one-off searches.
    pub const MAX_BYTES: usize = 256 * 1024;

    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            len: 0,
            clock: 0,
        }
    }

    /// Explicit diagnostic snapshot; does not allocate or change recency.
    pub fn census(&self) -> CacheCensus {
        let mut census = CacheCensus {
            entries: self.len,
            bucket_bytes: map_bytes(self.entries.capacity()),
            ..CacheCensus::default()
        };
        for bucket in self.entries.values() {
            census.bucket_bytes += bucket.capacity() * std::mem::size_of::<Entry>();
            for entry in bucket {
                census.key_bytes += entry.key.pattern.len()
                    + entry.key.whitespace.as_deref().map_or(0, <[u8]>::len);
                census.program_bytes += rc_bytes(&entry.regex);
            }
        }
        census.retained_bytes = census.program_bytes + census.key_bytes + census.bucket_bytes;
        census
    }

    pub fn get(
        &mut self,
        key: KeyRef,
        build: impl FnOnce() -> Result<Regex, RegexError>,
        st: &mut Stats,
    ) -> Result<Rc<Regex>, RegexError> {
        let hash = self.entries.hasher().hash_one(key);
        self.get_hashed(hash, key, build, st)
    }
    /// Return an existing program without compiling or retaining a missed key.
    pub fn lookup(&mut self, key: KeyRef, st: &mut Stats) -> Option<Rc<Regex>> {
        let hash = self.entries.hasher().hash_one(key);
        self.lookup_hashed(hash, key, st)
    }
    fn lookup_hashed(&mut self, hash: u64, key: KeyRef, st: &mut Stats) -> Option<Rc<Regex>> {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self
            .entries
            .get_mut(&hash)
            .and_then(|bucket| bucket.iter_mut().find(|entry| entry.key.as_ref() == key))
        {
            st.hits += 1;
            entry.used = self.clock;
            return Some(Rc::clone(&entry.regex));
        }
        None
    }
    fn get_hashed(
        &mut self,
        hash: u64,
        key: KeyRef,
        build: impl FnOnce() -> Result<Regex, RegexError>,
        st: &mut Stats,
    ) -> Result<Rc<Regex>, RegexError> {
        if let Some(program) = self.lookup_hashed(hash, key, st) {
            return Ok(program);
        }
        let re = Rc::new(build()?);
        st.compiles += 1;
        // Test the full one-entry footprint before evicting useful programs.
        let single_bytes = rc_bytes(&re)
            + key.pattern.len()
            + key.whitespace.map_or(0, <[u8]>::len)
            + std::mem::size_of::<Entry>()
            + map_bytes(3);
        if single_bytes > Self::MAX_BYTES {
            return Ok(re);
        }
        if self.len == Self::MAX_ENTRIES {
            self.evict_oldest();
        }
        let entry = Entry {
            key: Key::from(key),
            regex: Rc::clone(&re),
            used: self.clock,
        };
        match self.entries.entry(hash) {
            std::collections::hash_map::Entry::Occupied(mut bucket) => {
                let bucket = bucket.get_mut();
                bucket.push(entry);
                if bucket.capacity() > Self::MAX_ENTRIES {
                    bucket.shrink_to_fit();
                }
            }
            std::collections::hash_map::Entry::Vacant(bucket) => {
                bucket.insert(vec![entry]);
            }
        }
        self.len += 1;
        while self.census().retained_bytes > Self::MAX_BYTES {
            self.evict_oldest();
        }
        Ok(re)
    }
    fn evict_oldest(&mut self) {
        let (hash, index, _) = self
            .entries
            .iter()
            .flat_map(|(&hash, bucket)| {
                bucket
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (hash, index, entry.used))
            })
            .min_by_key(|&(_, _, used)| used)
            .unwrap();
        let bucket = self.entries.get_mut(&hash).unwrap();
        bucket.swap_remove(index);
        if bucket.is_empty() {
            self.entries.remove(&hash);
            // Rebuild after removing a hash, with the SAME hasher: stored hashes
            // are its output. This discards tombstones instead of letting long
            // miss/eviction churn grow a sparsely occupied backing table.
            let mut compact = HashMap::with_capacity_and_hasher(
                self.entries.len(),
                self.entries.hasher().clone(),
            );
            compact.extend(self.entries.drain());
            self.entries = compact;
        } else {
            // A formerly large collision chain must not pin its old capacity.
            bucket.shrink_to_fit();
        }
        self.len -= 1;
    }
}

fn rc_bytes(regex: &Regex) -> usize {
    // RcBox = strong + weak + aligned Regex. Account padding as well as counters.
    let header = 2 * std::mem::size_of::<usize>();
    let align = std::mem::align_of::<Regex>();
    let offset = header.next_multiple_of(align);
    (offset + std::mem::size_of::<Regex>())
        .next_multiple_of(align.max(std::mem::align_of::<usize>()))
        + regex.owned_bytes()
        - std::mem::size_of::<Regex>()
}

fn map_bytes(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    // The pinned std HashMap uses SwissTable: power-of-two buckets, <= 7/8
    // load, one control byte per raw bucket plus a SIMD group and alignment.
    // Our map has no tombstones (rebuilt on removal), so capacity identifies
    // its raw bucket count. Charge 64 extra bytes for group/alignment padding,
    // conservatively covering scalar/SSE/AVX groups on supported targets.
    // This is an explicit storage upper charge, NOT an allocator/RSS statistic;
    // revisit alongside a change of std's map implementation.
    let buckets = (capacity + 1).next_power_of_two();
    buckets * (std::mem::size_of::<(u64, Vec<Entry>)>() + 1) + 64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal_key(pattern: &[u8]) -> KeyRef<'_> {
        KeyRef {
            kind: Kind::Literal,
            pattern,
            pattern_multibyte: false,
            target_multibyte: false,
            posix: false,
            whitespace: None,
            case: None,
        }
    }

    #[test]
    fn empty_cache_has_no_bucket_allocation() {
        let mut cache = Cache::new();
        assert_eq!(cache.entries.capacity(), 0);
        assert!(
            cache
                .lookup(literal_key(b"missing"), &mut Stats::default())
                .is_none()
        );
        assert_eq!(cache.entries.capacity(), 0);
    }

    #[test]
    fn oversized_program_executes_without_displacing_or_retaining_entries() {
        let mut cache = Cache::new();
        let mut st = Stats::default();
        let small = cache
            .get(
                literal_key(b"a"),
                || Ok(Regex::literal(b"a", false, false, None)),
                &mut st,
            )
            .unwrap();
        let pattern = vec![b'x'; 256 * 1024];
        let large = cache
            .get(
                literal_key(&pattern),
                || Ok(Regex::literal(&pattern, false, false, None)),
                &mut st,
            )
            .unwrap();
        let hay = crate::Haystack {
            s1: &pattern,
            s2: &[],
            multibyte: false,
            point: None,
        };
        assert_eq!(
            large
                .matches(
                    &hay,
                    0,
                    pattern.len(),
                    &(),
                    &mut crate::Regs::default(),
                    &mut st
                )
                .unwrap(),
            Some(pattern.len())
        );
        assert_eq!(cache.len, 1);
        assert!(cache.lookup(literal_key(&pattern), &mut st).is_none());
        assert!(Rc::ptr_eq(
            &small,
            &cache.lookup(literal_key(b"a"), &mut st).unwrap()
        ));
    }

    #[test]
    fn byte_budget_evicts_lru_before_entry_limit_and_charges_keys() {
        let mut cache = Cache::new();
        let mut st = Stats::default();
        let patterns: Vec<_> = (0..4).map(|i| vec![b'a' + i; 32 * 1024]).collect();
        for pat in &patterns[..2] {
            cache
                .get(
                    literal_key(pat),
                    || Ok(Regex::literal(pat, false, false, None)),
                    &mut st,
                )
                .unwrap();
        }
        // Keep the first plan hot, so byte-pressure must discard the second.
        let hot = cache.lookup(literal_key(&patterns[0]), &mut st).unwrap();
        cache
            .get(
                literal_key(&patterns[2]),
                || Ok(Regex::literal(&patterns[2], false, false, None)),
                &mut st,
            )
            .unwrap();
        assert_eq!(cache.len, 2);
        assert!(cache.lookup(literal_key(&patterns[1]), &mut st).is_none());
        assert!(Rc::ptr_eq(
            &hot,
            &cache.lookup(literal_key(&patterns[0]), &mut st).unwrap()
        ));
        let census = cache.census();
        assert_eq!(census.key_bytes, 2 * patterns[0].len());
        assert_eq!(census.program_bytes, 2 * rc_bytes(&hot));
        assert!(census.bucket_bytes > 2 * std::mem::size_of::<Entry>());
        assert_eq!(
            census.retained_bytes,
            census.key_bytes + census.program_bytes + census.bucket_bytes
        );
        assert!(census.retained_bytes <= Cache::MAX_BYTES);
        // Whitespace is owned as part of a key, even for a tiny literal program.
        let whitespace = vec![b' '; Cache::MAX_BYTES];
        let mut key = literal_key(b"tiny");
        key.whitespace = Some(&whitespace);
        cache
            .get(
                key,
                || Ok(Regex::literal(b"tiny", false, false, None)),
                &mut st,
            )
            .unwrap();
        assert_eq!(cache.census(), census);
    }

    #[test]
    fn sparse_fold_capacity_can_make_a_short_key_uncacheable() {
        struct Fold;
        impl crate::CaseTable for Fold {
            fn canon(&self, c: u32) -> u32 {
                if (0x100..=0x102).contains(&c) {
                    0x100
                } else {
                    c
                }
            }
            fn eqv_next(&self, c: u32) -> u32 {
                match c {
                    0x100 => 0x101,
                    0x101 => 0x102,
                    0x102 => 0x100,
                    _ => c,
                }
            }
        }
        let pattern = [0xc4, 0x80].repeat(4096);
        assert!(pattern.len() < Cache::MAX_BYTES / 16);
        let mut key = literal_key(&pattern);
        key.pattern_multibyte = true;
        key.target_multibyte = true;
        key.case = Some((1, 1));
        let mut cache = Cache::new();
        let mut st = Stats::default();
        let program = cache
            .get(
                key,
                || Ok(Regex::literal(&pattern, true, true, Some(&Fold))),
                &mut st,
            )
            .unwrap();
        assert!(program.owned_bytes() > Cache::MAX_BYTES);
        let hay = crate::Haystack {
            s1: &pattern,
            s2: &[],
            multibyte: true,
            point: None,
        };
        assert_eq!(
            program
                .matches(
                    &hay,
                    0,
                    pattern.len(),
                    &(),
                    &mut crate::Regs::default(),
                    &mut st
                )
                .unwrap(),
            Some(pattern.len())
        );
        assert!(cache.lookup(key, &mut st).is_none());
        assert_eq!(cache.census(), CacheCensus::default());
    }

    #[test]
    fn churn_bounds_table_and_collision_capacities_and_preserves_lookup() {
        for collide in [false, true] {
            let mut cache = Cache::new();
            let mut st = Stats::default();
            for i in 0..2048u32 {
                let pattern = i.to_le_bytes();
                let key = literal_key(&pattern);
                if collide {
                    cache
                        .get_hashed(
                            0,
                            key,
                            || Ok(Regex::literal(&pattern, false, false, None)),
                            &mut st,
                        )
                        .unwrap();
                } else {
                    cache
                        .get(
                            key,
                            || Ok(Regex::literal(&pattern, false, false, None)),
                            &mut st,
                        )
                        .unwrap();
                }
                assert!(cache.len <= Cache::MAX_ENTRIES);
                assert!(cache.entries.capacity() <= 224);
                assert!(
                    cache
                        .entries
                        .values()
                        .all(|bucket| bucket.capacity() <= Cache::MAX_ENTRIES)
                );
                assert!(cache.census().retained_bytes <= Cache::MAX_BYTES);
            }
            assert_eq!(cache.len, Cache::MAX_ENTRIES);
            for i in 1920..2048u32 {
                let pattern = i.to_le_bytes();
                assert!(
                    if collide {
                        cache.lookup_hashed(0, literal_key(&pattern), &mut st)
                    } else {
                        cache.lookup(literal_key(&pattern), &mut st)
                    }
                    .is_some()
                );
            }
            // Emptying releases both the map and collision vector allocations;
            // external callers may still own programs, not charged to this cache.
            while cache.len > 0 {
                cache.evict_oldest();
            }
            assert_eq!(cache.census(), CacheCensus::default());
        }
    }

    #[test]
    fn lookup_misses_do_not_insert_and_hits_promote_colliding_entries() {
        let mut cache = Cache::new();
        let mut stats = Stats::default();
        let key = |pattern| KeyRef {
            kind: Kind::Literal,
            pattern,
            pattern_multibyte: false,
            target_multibyte: false,
            posix: false,
            whitespace: None,
            case: None,
        };
        assert!(cache.lookup_hashed(0, key(b"a"), &mut stats).is_none());
        assert_eq!(cache.len, 0);
        let a = cache
            .get_hashed(
                0,
                key(b"a"),
                || Ok(Regex::literal(b"a", false, false, None)),
                &mut stats,
            )
            .unwrap();
        let b = cache
            .get_hashed(
                0,
                key(b"b"),
                || Ok(Regex::literal(b"b", false, false, None)),
                &mut stats,
            )
            .unwrap();
        assert!(Rc::ptr_eq(
            &cache.lookup_hashed(0, key(b"a"), &mut stats).unwrap(),
            &a
        ));
        assert!(cache.lookup_hashed(0, key(b"c"), &mut stats).is_none());
        cache.evict_oldest();
        assert!(cache.lookup_hashed(0, key(b"b"), &mut stats).is_none());
        assert!(Rc::ptr_eq(
            &cache.lookup_hashed(0, key(b"a"), &mut stats).unwrap(),
            &a
        ));
        assert!(!Rc::ptr_eq(&a, &b));
        assert_eq!((cache.len, stats.compiles, stats.hits), (1, 2, 2));
        let c = cache
            .get(
                key(b"c"),
                || Ok(Regex::literal(b"c", false, false, None)),
                &mut stats,
            )
            .unwrap();
        assert!(Rc::ptr_eq(
            &cache.lookup(key(b"c"), &mut stats).unwrap(),
            &c
        ));
    }

    #[test]
    fn colliding_keys_remain_distinct() {
        let mut cache = Cache::new();
        let mut stats = Stats::default();
        let mut programs = Vec::new();
        for pattern in [b"a", b"b", b"a", b"b"] {
            programs.push(
                cache
                    .get_hashed(
                        0,
                        KeyRef {
                            kind: Kind::Literal,
                            pattern,
                            pattern_multibyte: false,
                            target_multibyte: false,
                            posix: false,
                            whitespace: None,
                            case: None,
                        },
                        || Ok(Regex::literal(pattern, false, false, None)),
                        &mut stats,
                    )
                    .unwrap(),
            );
        }
        assert!(!Rc::ptr_eq(&programs[0], &programs[1]));
        assert!(Rc::ptr_eq(&programs[0], &programs[2]));
        assert!(Rc::ptr_eq(&programs[1], &programs[3]));
        assert_eq!((stats.compiles, stats.hits), (2, 2));
    }
}
