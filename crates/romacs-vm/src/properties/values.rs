//! One character's properties as GNU's interval plist holds them: newest
//! first, each key once.  A new key goes in front (`add_properties`
//! conses it on), a new value for a key keeps its place (`Fsetcar`), and
//! two are equal when they hold the same keys with the same values in any
//! order, as `intervals_equal` compares them.
use super::Key;
use crate::Value;

#[derive(Clone, Debug, Default)]
pub(crate) struct Values(Vec<(Key, Value)>);
romacs_snapshot::snapshot_tuple!(Values(f0));

impl Values {
    pub(crate) fn get(&self, key: Key) -> Option<&Value> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
    pub(crate) fn contains_key(&self, key: Key) -> bool {
        self.get(key).is_some()
    }
    /// What an insertion that inherits INHERITED holds where its own text
    /// had OWN, as graft_intervals_into_buffer's merge_properties leaves
    /// it: INHERITED, with each of OWN's keys it lacks consed on in OWN's
    /// order.
    pub(crate) fn inheriting(inherited: &[(Key, Value)], own: &Self) -> Self {
        let mut values: Self = inherited.iter().copied().collect();
        for &(key, value) in &own.0 {
            if !values.contains_key(key) {
                values.insert(key, value);
            }
        }
        values
    }
    /// KEY's value is now VALUE, in KEY's place, or in front for a new key.
    pub(crate) fn insert(&mut self, key: Key, value: Value) -> Option<Value> {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, old)) => Some(std::mem::replace(old, value)),
            None => {
                self.0.insert(0, (key, value));
                None
            }
        }
    }
    /// The pairs, newest first.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = (&Key, &Value)> {
        self.0.iter().map(|(k, v)| (k, v))
    }
    pub(crate) fn keys(&self) -> impl Iterator<Item = &Key> {
        self.0.iter().map(|(k, _)| k)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
    /// Each key's symbol and each value through VISIT, in place: a key
    /// keeps its symbol alive, and comes back the symbol it now is.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for (key, value) in &mut self.0 {
            visit(&mut key.0);
            visit(value);
        }
    }
}
/// PAIRS newest first, as a plist reads; a repeated key keeps its first
/// value, as a plist lookup finds it.
impl FromIterator<(Key, Value)> for Values {
    fn from_iter<I: IntoIterator<Item = (Key, Value)>>(pairs: I) -> Self {
        let mut out = Vec::new();
        for (key, value) in pairs {
            if !out.iter().any(|(k, _)| *k == key) {
                out.push((key, value));
            }
        }
        Self(out)
    }
}
impl IntoIterator for Values {
    type Item = (Key, Value);
    type IntoIter = std::vec::IntoIter<(Key, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a Values {
    type Item = (&'a Key, &'a Value);
    type IntoIter = std::iter::Map<std::slice::Iter<'a, (Key, Value)>, fn(&'a (Key, Value)) -> (&'a Key, &'a Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(k, v)| (k, v))
    }
}
impl PartialEq for Values {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(&k, v)| other.get(k) == Some(v))
    }
}
impl Eq for Values {}
