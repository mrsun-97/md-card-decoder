//! Brute-force reference solver for tests: no memo, no action reduction, no pruning, every guess
//! (re-guesses included). Exponential — tiny pools only.

use super::{Continuation, Node, challenge_reward};
use crate::belief::Belief;
use crate::ctx::Ctx;
use crate::data::{ALL_FIELDS, Field, Gid};
use crate::kernel::Partitioner;
use crate::value::Val;

pub fn value(ctx: &Ctx, node: &Node, cont: &dyn Continuation) -> Val {
    let mut part = Partitioner::default();
    visit(ctx, node, cont, &mut part)
}

fn visit(ctx: &Ctx, n: &Node, cont: &dyn Continuation, part: &mut Partitioner) -> Val {
    if n.challenges == 0 || n.cands.is_empty() {
        return Val::ZERO;
    }
    let total = ctx.mass(&n.cands) as f64;
    let mut best: Option<Val> = None;
    let unknown = !n.known & ALL_FIELDS;
    if n.hints > 0 && unknown != 0 {
        let fields: Vec<Field> = Field::ALL.into_iter().filter(|f| unknown & f.bit() != 0).collect();
        let mut v = Val::ZERO;
        for &f in &fields {
            let mut values: Vec<u8> = n.cands.iter().map(|&g| ctx.table.shown(f, g)).collect();
            values.sort_unstable();
            values.dedup();
            for s in values {
                let sub: Vec<Gid> = n.cands.iter().copied().filter(|&g| ctx.table.shown(f, g) == s).collect();
                let p = ctx.mass(&sub) as f64 / total / fields.len() as f64;
                let child = Node { cands: sub, known: n.known | f.bit(), hints: n.hints - 1, ..n.clone() };
                v += visit(ctx, &child, cont, part) * p;
            }
        }
        best = Some(v + Val::new(0.0, 0.0, -1.0));
    }
    let (mut cells, mut arena) = (Vec::new(), Vec::new());
    for &g in &ctx.pool_groups {
        cells.clear();
        arena.clear();
        part.split(ctx, g, &n.cands, &mut cells, &mut arena);
        let mut v = Val::ZERO;
        for c in cells.clone() {
            let p = c.mass as f64 / total;
            let mask = c.fb.mask();
            let mut branch = challenge_reward(n.matched, mask, c.fb.solved());
            if c.fb.solved() {
                branch += cont.value(n.hints, n.challenges - 1);
            } else {
                let sub = arena[c.start as usize..(c.start + c.count) as usize].to_vec();
                let child = Node {
                    cands: sub,
                    known: n.known | mask,
                    matched: n.matched | mask,
                    challenges: n.challenges - 1,
                    ..n.clone()
                };
                branch += visit(ctx, &child, cont, part);
            }
            v += branch * p;
        }
        if best.is_none_or(|b| v.gt(&b)) {
            best = Some(v);
        }
    }
    best.unwrap_or(Val::ZERO)
}

/// Node for a belief with the given resources.
pub fn node_of(b: &Belief, hints: u8, challenges: u8) -> Node {
    Node { cands: b.cands.clone(), known: b.known, matched: b.matched, hints, challenges }
}
