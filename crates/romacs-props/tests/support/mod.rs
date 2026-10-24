use romacs_chars::CharPos;
use romacs_props::{PropSets, Props, SetId, Sticky, StickyEnv, StickyKey};
use std::cell::RefCell;
pub const FRONT: u64 = 4;
pub const REAR: u64 = 5;
#[derive(Default)]
pub struct Env {
    pub lists: RefCell<Vec<Vec<u64>>>,
    pub defaults: Vec<(u64, bool)>,
}
impl Env {
    pub fn list(&self, keys: Vec<u64>) -> u64 {
        let mut t = self.lists.borrow_mut();
        t.push(keys);
        1000 + t.len() as u64 - 1
    }
    pub fn materialize(&self, s: &mut PropSets<u64>, x: Sticky<u64>) -> SetId {
        match x {
            Sticky::Set(id) => id,
            Sticky::Build {
                front,
                rear,
                mut pairs,
            } => {
                if !rear.is_empty() {
                    pairs.insert(0, (REAR, self.list(rear)));
                }
                if !front.is_empty() {
                    pairs.insert(0, (FRONT, self.list(front)));
                }
                s.intern(&pairs)
            }
        }
    }
    pub fn inherited(&self, p: &Props, s: &mut PropSets<u64>, at: usize) -> SetId {
        let x = p.sticky_at(s, CharPos(at), self);
        self.materialize(s, x)
    }
}
impl StickyEnv<u64> for Env {
    fn textget(&self, p: &[(u64, u64)], k: StickyKey) -> u64 {
        p.iter()
            .find(|x| {
                x.0 == match k {
                    StickyKey::Front => FRONT,
                    StickyKey::Rear => REAR,
                }
            })
            .map_or_else(
                || {
                    if matches!(k, StickyKey::Front) && p.contains(&(6, 7)) {
                        1
                    } else {
                        0
                    }
                },
                |x| x.1,
            )
    }
    fn tmem(&self, k: u64, v: u64) -> bool {
        if v >= 1000 {
            self.lists.borrow()[(v - 1000) as usize].contains(&k)
        } else {
            v != 0
        }
    }
    fn is_nil(&self, v: u64) -> bool {
        v == 0
    }
    fn is_cons(&self, v: u64) -> bool {
        v >= 1000
    }
    fn default_nonsticky(&self, k: u64) -> Option<bool> {
        self.defaults.iter().find(|p| p.0 == k).map(|p| p.1)
    }
    fn category_front_sticky_t(&self, p: &[(u64, u64)]) -> bool {
        p.contains(&(6, 7))
    }
    fn is_sticky_key(&self, k: u64) -> bool {
        k == FRONT || k == REAR
    }
}
