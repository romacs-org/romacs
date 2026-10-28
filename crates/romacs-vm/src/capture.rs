//! Binder-resolved capture mutability. Registers identify bindings after macro
//! expansion; spelling is never used to connect a parent and a child environment.
use super::*;

#[cfg(test)]
thread_local! {
    static INDEX_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Bindings {
    mutated: Vec<bool>,
    children: Vec<Vec<(usize, usize)>>,
    unboxed: Vec<bool>,
}

impl Bindings {
    fn index(code: &[Op], sites: &[Vec<usize>], regs: usize) -> Self {
        #[cfg(test)]
        INDEX_WORK.with(|work| {
            work.set(work.get() + code.len() + regs + sites.iter().map(Vec::len).sum::<usize>())
        });
        let mut index = Self {
            mutated: vec![false; regs],
            children: vec![Vec::new(); regs],
            unboxed: vec![false; regs],
        };
        for &op in code {
            match op {
                Op::WriteCell(reg, _) => index.mutated[reg] = true,
                Op::Closure(_, function, site) => {
                    for (capture, &source) in sites[site].iter().enumerate() {
                        index.children[source].push((function, capture));
                    }
                }
                _ => {}
            }
        }
        index
    }

    fn rewrite(&self, code: &mut [Op]) {
        for op in code {
            *op = match *op {
                Op::Cell(reg) if self.unboxed[reg] => Op::Move(reg, reg),
                Op::ReadCell(dest, source) if self.unboxed[source] => Op::Move(dest, source),
                Op::WriteCell(dest, source) if self.unboxed[dest] => Op::Move(dest, source),
                other => other,
            };
        }
    }
}

impl Vm {
    fn binding_summary(&self, mut index: Bindings) -> Bindings {
        for (reg, children) in index.children.iter().enumerate() {
            for &(id, capture) in children {
                index.mutated[reg] |= self.functions[id].capture_mutations[capture];
            }
        }
        index
    }

    pub(super) fn classify_bindings(&mut self, b: &mut Builder) -> Vec<bool> {
        // Installed children retain their transitive mutation summaries. Consult
        // only direct edges here; index descendants lazily only when unboxing
        // reaches them, then optimize each changed function once.
        // Shared mutable captures retain location semantics after macro expansion.
        let mut functions = HashMap::default();
        let mut root = self.binding_summary(Bindings::index(&b.code, &b.sites, b.next));
        let mut pending = Vec::new();
        for &op in &b.code {
            if let Op::Cell(reg) = op
                && (root.children[reg].is_empty() || !root.mutated[reg])
            {
                root.unboxed[reg] = true;
                pending.extend(root.children[reg].iter().copied());
            }
        }
        while let Some((id, reg)) = pending.pop() {
            let index = functions.entry(id).or_insert_with(|| {
                let f = &self.functions[id];
                Bindings::index(&f.code, &f.sites, f.regs)
            });
            if !index.unboxed[reg] {
                index.unboxed[reg] = true;
                pending.extend(index.children[reg].iter().copied());
            }
        }
        root.rewrite(&mut b.code);
        for (id, index) in functions {
            if index.unboxed.iter().any(|&unboxed| unboxed) {
                let f = Arc::make_mut(&mut self.functions[id]);
                index.rewrite(&mut f.code);
                f.code = optimize(std::mem::take(&mut f.code), &f.sites, f.regs);
            }
        }
        // Rewriting never changes mutation of a captured slot: it can only
        // unbox a capture whose sole source binder was proved immutable.
        root.mutated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_index_work_scales_linearly_in_lambda_depth() {
        let mut previous = None;
        for depth in [5, 10, 20, 40, 80] {
            let mut source = "x".to_owned();
            for _ in 0..depth {
                source = format!("(funcall (lambda () {source}))");
            }
            let source = format!("(let ((x 7)) {source})");
            let mut vm = Vm::new();
            INDEX_WORK.with(|work| work.set(0));
            let entry = vm.compile_source(&source).unwrap();
            let work = INDEX_WORK.with(|work| work.get());
            assert_eq!(vm.execute_raw(entry).unwrap().as_integer().unwrap(), 7);
            if let Some(prior) = previous {
                assert!(
                    work <= 2 * prior,
                    "depth {depth}: index work {work} exceeds twice {prior}"
                );
            }
            previous = Some(work);
        }
    }
}
