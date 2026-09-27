//! Depth-limited expectimax over a fixed action set (inherited by every node), with a pluggable
//! leaf value at the horizon and a pluggable cross-puzzle continuation at solves.
//!
//! `depth` counts actions: at depth 1 every child is a leaf, so children are scored from their
//! cell mass and entropy without materialising candidate sets. Every non-root node may also
//! "stop" (value 0), which lets a truncated search decline useless actions.

use std::rc::Rc;

use rustc_hash::FxHashMap;

use super::{Action, Continuation, Node, challenge_reward};
use crate::cont::{Leaf, LeafState};
use crate::ctx::{Ctx, set_entropy};
use crate::data::{ALL_FIELDS, Field, Gid};
use crate::kernel::{Cell, CellE, Partitioner};
use crate::value::Val;

pub struct Horizon<'a> {
    ctx: &'a Ctx,
    cont: &'a dyn Continuation,
    leaf: &'a dyn Leaf,
    actions: Rc<[Gid]>,
    part: Partitioner,
    memo: FxHashMap<(u128, u32), Val>,
    /// Nodes expanded (for budgets and diagnostics).
    pub nodes: u64,
}

const HINT_COST: Val = Val::new(0.0, 0.0, -1.0);

impl<'a> Horizon<'a> {
    pub fn new(
        ctx: &'a Ctx,
        cont: &'a dyn Continuation,
        leaf: &'a dyn Leaf,
        actions: Vec<Gid>,
    ) -> Horizon<'a> {
        Horizon {
            ctx,
            cont,
            leaf,
            actions: actions.into(),
            part: Partitioner::default(),
            memo: FxHashMap::default(),
            nodes: 0,
        }
    }

    /// Q-value of each root action (HINT first when available). No STOP at the root.
    pub fn root(&mut self, n: &Node, depth: u8) -> Vec<(Action, Val)> {
        let mut out = Vec::new();
        self.expand(n, depth.max(1), Some(&mut out));
        out
    }

    fn visit(&mut self, n: &Node, depth: u8) -> Val {
        if n.challenges == 0 || n.cands.is_empty() {
            return Val::ZERO;
        }
        if depth == 0 {
            return self.leaf_of(n);
        }
        let key = (
            self.ctx.set_hash(&n.cands),
            n.known as u32
                | (n.matched as u32) << 6
                | (n.hints as u32) << 12
                | (n.challenges as u32) << 18
                | (depth as u32) << 26,
        );
        if let Some(&v) = self.memo.get(&key) {
            return v;
        }
        let v = self.expand(n, depth, None);
        self.memo.insert(key, v);
        v
    }

    fn leaf_of(&self, n: &Node) -> Val {
        let mass = self.ctx.mass(&n.cands);
        self.leaf.eval(&LeafState {
            mass,
            entropy: self.ctx.entropy(&n.cands),
            known: n.known,
            matched: n.matched,
            hints: n.hints,
            challenges: n.challenges,
        })
    }

    fn expand(&mut self, n: &Node, depth: u8, mut record: Option<&mut Vec<(Action, Val)>>) -> Val {
        self.nodes += 1;
        let ctx = self.ctx;
        let total = ctx.mass(&n.cands) as f64;
        // STOP (value 0) everywhere except the root.
        let mut best = if record.is_some() { None } else { Some(Val::ZERO) };
        let unknown = !n.known & ALL_FIELDS;
        if n.hints > 0 && unknown != 0 {
            let v = self.hint_value(n, depth, total) + HINT_COST;
            if let Some(r) = record.as_deref_mut() {
                r.push((Action::Hint, v));
            }
            if best.is_none_or(|b| v.gt(&b)) {
                best = Some(v);
            }
        }
        let actions = Rc::clone(&self.actions);
        if depth == 1 {
            let mut cells: Vec<CellE> = Vec::new();
            for &g in actions.iter() {
                self.part.cells_ent(ctx, g, &n.cands, &mut cells);
                if cells.len() == 1 && !cells[0].fb.solved() && cells[0].fb.mask() & !n.matched == 0 {
                    continue;
                }
                let mut v = Val::ZERO;
                for c in &cells {
                    let mask = c.fb.mask();
                    let mut branch = challenge_reward(n.matched, mask, c.fb.solved());
                    branch += if c.fb.solved() {
                        self.cont.value(n.hints, n.challenges - 1)
                    } else {
                        self.leaf.eval(&LeafState {
                            mass: c.mass,
                            entropy: set_entropy(c.mass, c.wlw),
                            known: n.known | mask,
                            matched: n.matched | mask,
                            hints: n.hints,
                            challenges: n.challenges - 1,
                        })
                    };
                    v += branch * (c.mass as f64 / total);
                }
                if let Some(r) = record.as_deref_mut() {
                    r.push((Action::Challenge(g), v));
                }
                if best.is_none_or(|b| v.gt(&b)) {
                    best = Some(v);
                }
            }
        } else {
            let (mut cells, mut arena): (Vec<Cell>, Vec<Gid>) = (Vec::new(), Vec::new());
            for &g in actions.iter() {
                cells.clear();
                arena.clear();
                self.part.split(ctx, g, &n.cands, &mut cells, &mut arena);
                if cells.len() == 1 && !cells[0].fb.solved() && cells[0].fb.mask() & !n.matched == 0 {
                    continue;
                }
                let mut v = Val::ZERO;
                for c in cells.clone() {
                    let mask = c.fb.mask();
                    let mut branch = challenge_reward(n.matched, mask, c.fb.solved());
                    branch += if c.fb.solved() {
                        self.cont.value(n.hints, n.challenges - 1)
                    } else {
                        let child = Node {
                            cands: arena[c.start as usize..(c.start + c.count) as usize].to_vec(),
                            known: n.known | mask,
                            matched: n.matched | mask,
                            hints: n.hints,
                            challenges: n.challenges - 1,
                        };
                        self.visit(&child, depth - 1)
                    };
                    v += branch * (c.mass as f64 / total);
                }
                if let Some(r) = record.as_deref_mut() {
                    r.push((Action::Challenge(g), v));
                }
                if best.is_none_or(|b| v.gt(&b)) {
                    best = Some(v);
                }
            }
        }
        best.unwrap_or(Val::ZERO)
    }

    /// Expected value after a hint (field uniform over unknown fields), before its action cost.
    fn hint_value(&mut self, n: &Node, depth: u8, total: f64) -> Val {
        let ctx = self.ctx;
        let unknown = !n.known & ALL_FIELDS;
        let fields: Vec<Field> = Field::ALL.into_iter().filter(|f| unknown & f.bit() != 0).collect();
        let mut v = Val::ZERO;
        for &f in &fields {
            let mut by_value: Vec<(u8, Gid)> = n.cands.iter().map(|&g| (ctx.table.shown(f, g), g)).collect();
            by_value.sort_unstable();
            let mut i = 0;
            while i < by_value.len() {
                let s = by_value[i].0;
                let start = i;
                while i < by_value.len() && by_value[i].0 == s {
                    i += 1;
                }
                let mut sub: Vec<Gid> = by_value[start..i].iter().map(|&(_, g)| g).collect();
                sub.sort_unstable();
                let mass = ctx.mass(&sub);
                let p = mass as f64 / total / fields.len() as f64;
                let child = Node { cands: sub, known: n.known | f.bit(), hints: n.hints - 1, ..n.clone() };
                v += self.visit(&child, depth - 1) * p;
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::belief::{Belief, Event};
    use crate::cont::ZeroLeaf;
    use crate::data::Pool;
    use crate::rng::Rng;
    use crate::rules::RulePolicy;
    use crate::search::reference;
    use crate::search::{LegacyContinuation, NoContinuation};
    use crate::synth::random_table;

    /// With every action and enough depth, the horizon search is the exact optimum.
    #[test]
    fn full_depth_equals_brute_force() {
        let mut rng = Rng::new(3);
        for round in 0..100 {
            let ctx =
                Ctx::new(random_table(&mut rng, 4 + round % 5), RulePolicy::default(), Pool::MasterDuel);
            let all = ctx.pool_groups.clone();
            let target = all[rng.below(all.len() as u64) as usize];
            let mut b = Belief::prior(&ctx);
            let f = Field::from_index(rng.below(6) as usize);
            b.apply(&ctx, &Event::Reveal { field: f, shown: ctx.table.shown(f, target) }).unwrap();
            let (h, c) = (rng.below(3) as u8, 1 + rng.below(3) as u8);
            let node = reference::node_of(&b, h, c);
            let legacy = LegacyContinuation { remaining: 2 };
            let conts: [&dyn Continuation; 2] = [&NoContinuation, &legacy];
            for cont in conts {
                let want = reference::value(&ctx, &node, cont);
                let mut hz = Horizon::new(&ctx, cont, &ZeroLeaf, all.clone());
                let got =
                    hz.root(&node, h + c).into_iter().map(|(_, v)| v).fold(None::<Val>, |a, v| match a {
                        Some(x) if !v.gt(&x) => Some(x),
                        _ => Some(v),
                    });
                assert_eq!(got.unwrap().cmp_eps(&want), std::cmp::Ordering::Equal, "round {round}");
            }
        }
    }
}
