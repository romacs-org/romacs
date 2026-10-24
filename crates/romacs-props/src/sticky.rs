use crate::{PropSets, Props, SetId};
use romacs_chars::CharPos;
use std::hash::Hash;
#[derive(Clone, Copy, Debug)]
pub enum StickyKey {
    Front,
    Rear,
}
pub trait StickyEnv<V> {
    fn textget(&self, pairs: &[(V, V)], which: StickyKey) -> V;
    fn tmem(&self, sym: V, set: V) -> bool;
    fn is_nil(&self, v: V) -> bool;
    fn is_cons(&self, v: V) -> bool;
    fn default_nonsticky(&self, sym: V) -> Option<bool>;
    fn category_front_sticky_t(&self, pairs: &[(V, V)]) -> bool;
    fn is_sticky_key(&self, k: V) -> bool;
}
#[derive(Debug)]
pub enum Sticky<V> {
    Set(SetId),
    Build {
        front: Vec<V>,
        rear: Vec<V>,
        pairs: Vec<(V, V)>,
    },
}
impl Props {
    /// Pure GNU `adjust_intervals_for_insertion` / `merge_properties_sticky`.
    /// Build asks the owner to materialize fresh front/rear lists, in that order.
    pub fn sticky_at<V: Copy + Eq + Hash>(
        &self,
        sets: &mut PropSets<V>,
        at: CharPos,
        env: &impl StickyEnv<V>,
    ) -> Sticky<V> {
        debug_assert!(at.0 <= self.len());
        let left = if at.0 > 0 {
            self.run_at(at - 1).2
        } else {
            SetId::EMPTY
        };
        let right = if at.0 < self.len() {
            self.run_at(at).2
        } else {
            SetId::EMPTY
        };
        if at.0 > 0 && at.0 < self.len() && self.run_at(at).0 < at {
            let p = sets.pairs(left);
            let rear = env.textget(p, StickyKey::Rear);
            let front = env.textget(p, StickyKey::Front);
            let problem = if !env.is_cons(rear) && !env.is_nil(rear) {
                true
            } else if !env.is_cons(front) && !env.is_nil(front) {
                false
            } else {
                p.iter().any(|&(k, _)| {
                    !env.tmem(k, front) && (env.tmem(k, rear) || env.default_nonsticky(k).is_some())
                })
            };
            if !problem {
                return Sticky::Set(left);
            }
        }
        let lp = sets.pairs(left);
        let rp = sets.pairs(right);
        let lf = env.textget(lp, StickyKey::Front);
        let lr = env.textget(lp, StickyKey::Rear);
        let rf = env.textget(rp, StickyKey::Front);
        let rr = env.textget(rp, StickyKey::Rear);
        let mut front = Vec::new();
        let mut rear = Vec::new();
        let mut pairs = Vec::new();
        for &(k, rv) in rp {
            if env.is_sticky_key(k) {
                continue;
            }
            let lv = lp.iter().find(|p| p.0 == k).map(|p| p.1);
            let default = env.default_nonsticky(k);
            let mut use_left = lv.is_some() && !env.tmem(k, lr) && default != Some(true);
            let mut use_right = env.tmem(k, rf) || default == Some(false);
            if use_left && use_right {
                if env.is_nil(lv.unwrap()) {
                    use_left = false;
                } else if env.is_nil(rv) {
                    use_right = false;
                }
            }
            let chosen = if use_left {
                Some((lv.unwrap(), lf, lr))
            } else if use_right {
                Some((rv, rf, rr))
            } else {
                None
            };
            if let Some((v, f, r)) = chosen {
                pairs.push((k, v));
                if env.tmem(k, f) {
                    front.push(k);
                }
                if env.tmem(k, r) {
                    rear.push(k);
                }
            }
        }
        for &(k, v) in lp {
            if env.is_sticky_key(k) || rp.iter().any(|p| p.0 == k) {
                continue;
            }
            let default = env.default_nonsticky(k);
            if !env.tmem(k, lr) && default != Some(true) {
                pairs.push((k, v));
                if env.tmem(k, lf) {
                    front.push(k);
                }
            } else if env.tmem(k, rf) || default == Some(false) {
                front.push(k);
                if env.tmem(k, rr) {
                    rear.push(k);
                }
            }
        }
        if env.category_front_sticky_t(&pairs) {
            front.clear();
        }
        if front.is_empty() && rear.is_empty() {
            let id = sets.intern(&pairs);
            // GNU joins left before right when the computed plist is intervals_equal.
            return Sticky::Set(if sets.set_eq(id, left) {
                left
            } else if sets.set_eq(id, right) {
                right
            } else {
                id
            });
        }
        Sticky::Build { front, rear, pairs }
    }
}
