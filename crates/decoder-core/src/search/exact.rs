//! Exact expectimax within one puzzle, for candidate sets of at most 64 groups.
//!
//! - Candidates get root-local indices; every node's set is a `u64` bitset and the memo key is a
//!   collision-free `u128` (set, known, matched, hints, challenges) — no guessed set, no strings.
//! - The feedback of every root action against every local candidate is precomputed once.
//! - Actions are reduced per node to one per distinct outcome (same child sets, same rewarded
//!   match bits); children inherit the reduced list. No-op challenges are skipped.
//! - Branch-and-bound with admissible upper bounds (PLAN.md §4.6, research/09 §2.3):
//!   * P̄₁ — the exact maximum probability of solving with the next challenge, using every
//!     remaining hint first (information cannot hurt; the revealed fields are a uniform subset);
//!   * solved: U_S = P̄₁·g(1) if c = 1, else P̄₁·(g(1) − g(2)) + g(2), with g(j) = 1 + F_S(h, c − j);
//!   * matches: U_M = (6 − |matched|) + the same combination of the continuation's match part;
//!   * actions: E[#actions] ≥ 2 − P̄₁(no hints) when c ≥ 2, else ≥ 1.
//!
//!   An action is skipped when its upper bound is lexicographically below the best exact value
//!   found so far; a node stops when the best value reaches the node's upper bound.
//! - With zero continuation (last puzzle) resources are capped: hints ≤ unknown fields,
//!   challenges ≤ |C| + 6 − |matched| (a no-op-free policy never uses more).

use std::cmp::Ordering;

use rustc_hash::FxHashMap;

use super::{Action, Continuation, Node, challenge_reward};
use crate::ctx::Ctx;
use crate::data::{ALL_FIELDS, Field, Gid};
use crate::value::Val;

/// The search stopped because its state budget ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfBudget;

/// The candidate set is too large for the exact tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LNode {
    set: u64,
    known: u8,
    matched: u8,
    hints: u8,
    challenges: u8,
}

impl LNode {
    #[inline]
    fn key(&self) -> u128 {
        (self.set as u128)
            | (self.known as u128) << 64
            | (self.matched as u128) << 70
            | (self.hints as u128) << 76
            | (self.challenges as u128) << 84
    }
}

/// One outcome cell of a local partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LCell {
    set: u64,
    mask: u8,
    solved: bool,
    mass: u64,
}

/// Result of a root evaluation for one action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootQ {
    pub action: Action,
    /// Exact Q-value, or `None` when the action was proven worse than the best (pruned).
    pub value: Option<Val>,
    /// Upper bound of the Q-value (equals `value` when exact).
    pub upper: Val,
}

pub struct ExactSolver<'a> {
    cont: &'a dyn Continuation,
    /// Root-local index → group.
    cands: Vec<Gid>,
    w: Vec<u64>,
    /// Root actions (guess groups) and their feedback against each local candidate.
    actions: Vec<Gid>,
    fb: Vec<u16>,
    /// Shown value per field per local candidate.
    shown: [Vec<u8>; 6],
    /// For each local candidate, indices into `solver_ids` of the guesses that fully match it.
    solvers: Vec<Vec<u16>>,
    solver_count: usize,
    acc: Vec<u64>,
    memo: FxHashMap<u128, Val>,
    pbar_memo: FxHashMap<(u64, u8, u8), f64>,
    /// Nodes expanded so far (memo misses).
    pub expanded: u64,
    /// Maximum nodes to expand before giving up.
    pub budget: u64,
    /// Actions skipped by branch-and-bound.
    pub pruned: u64,
    /// Disable branch-and-bound (reference runs in tests).
    pub no_bounds: bool,
}

impl<'a> ExactSolver<'a> {
    /// Prepare a solver for candidate set `cands` (≤ 64 groups); `guesses` is the action universe.
    pub fn new(
        ctx: &'a Ctx,
        cont: &'a dyn Continuation,
        cands: &[Gid],
        guesses: &[Gid],
        budget: u64,
    ) -> Result<ExactSolver<'a>, TooLarge> {
        if cands.len() > 64 {
            return Err(TooLarge);
        }
        let w: Vec<u64> = cands.iter().map(|&g| ctx.weights[g as usize] as u64).collect();
        let mut shown: [Vec<u8>; 6] = Default::default();
        for f in Field::ALL {
            shown[f as usize] = cands.iter().map(|&g| ctx.table.shown(f, g)).collect();
        }
        // Keep one guess per distinct feedback column at the root (exact reduction).
        let mut seen: FxHashMap<Vec<u16>, ()> = FxHashMap::default();
        let mut actions = Vec::new();
        let mut fb = Vec::new();
        for &g in guesses {
            let row = ctx.rules.row(&ctx.table, g);
            let col: Vec<u16> = cands.iter().map(|&t| row.feedback_packed(&ctx.rules, t).0).collect();
            if seen.insert(col.clone(), ()).is_none() {
                actions.push(g);
                fb.extend_from_slice(&col);
            }
        }
        let mut solver_index: FxHashMap<Gid, u16> = FxHashMap::default();
        let solvers: Vec<Vec<u16>> = cands
            .iter()
            .map(|&t| {
                ctx.full_matches(t)
                    .iter()
                    .map(|&g| {
                        let next = solver_index.len() as u16;
                        *solver_index.entry(g).or_insert(next)
                    })
                    .collect()
            })
            .collect();
        let solver_count = solver_index.len();
        Ok(ExactSolver {
            cont,
            cands: cands.to_vec(),
            w,
            actions,
            fb,
            shown,
            solvers,
            solver_count,
            acc: vec![0; solver_count],
            memo: FxHashMap::default(),
            pbar_memo: FxHashMap::default(),
            expanded: 0,
            budget,
            pruned: 0,
            no_bounds: false,
        })
    }

    fn full_set(&self) -> u64 {
        if self.cands.len() == 64 { u64::MAX } else { (1u64 << self.cands.len()) - 1 }
    }

    fn lnode(&self, n: &Node) -> LNode {
        assert_eq!(n.cands, self.cands, "node must use the solver's candidate set");
        self.normalize(LNode {
            set: self.full_set(),
            known: n.known,
            matched: n.matched,
            hints: n.hints,
            challenges: n.challenges,
        })
    }

    /// Optimal value of the root node.
    pub fn value(&mut self, n: &Node) -> Result<Val, OutOfBudget> {
        let root = self.lnode(n);
        let all: Vec<u16> = (0..self.actions.len() as u16).collect();
        self.visit(root, &all)
    }

    /// Exact Q-values of all root actions (HINT first when available); branch-and-bound prunes
    /// only below the root.
    pub fn root(&mut self, n: &Node) -> Result<Vec<RootQ>, OutOfBudget> {
        let root = self.lnode(n);
        let all: Vec<u16> = (0..self.actions.len() as u16).collect();
        let mut out = Vec::new();
        self.expand(root, &all, Some(&mut out))?;
        Ok(out)
    }

    fn normalize(&self, mut n: LNode) -> LNode {
        if self.cont.is_zero() {
            let unknown = (!n.known & ALL_FIELDS).count_ones() as u8;
            n.hints = n.hints.min(unknown);
            let useful = n.set.count_ones() as usize + 6 - n.matched.count_ones() as usize;
            n.challenges = n.challenges.min(useful.min(255) as u8);
        }
        n
    }

    #[inline]
    fn mass(&self, set: u64) -> u64 {
        let mut s = set;
        let mut m = 0;
        while s != 0 {
            m += self.w[s.trailing_zeros() as usize];
            s &= s - 1;
        }
        m
    }

    /// Cells of action `a` on `set`, sorted by set (canonical order).
    fn partition(&self, a: u16, set: u64, out: &mut Vec<LCell>) {
        out.clear();
        let n = self.cands.len();
        let col = &self.fb[a as usize * n..(a as usize + 1) * n];
        let mut codes: [(u16, u64, u64); 64] = [(0, 0, 0); 64];
        let mut k = 0;
        let mut s = set;
        while s != 0 {
            let i = s.trailing_zeros() as usize;
            s &= s - 1;
            let code = col[i];
            let pos = codes[..k].iter().position(|c| c.0 == code);
            let j = pos.unwrap_or_else(|| {
                codes[k] = (code, 0, 0);
                k += 1;
                k - 1
            });
            codes[j].1 |= 1 << i;
            codes[j].2 += self.w[i];
        }
        for &(code, cs, m) in &codes[..k] {
            let fb = crate::rules::Fb(code);
            out.push(LCell { set: cs, mask: fb.mask(), solved: fb.solved(), mass: m });
        }
        out.sort_unstable();
    }

    /// Cells of a hint on field `f`.
    fn hint_cells(&self, f: Field, set: u64, out: &mut Vec<(u64, u64)>) {
        out.clear();
        let col = &self.shown[f as usize];
        let mut s = set;
        while s != 0 {
            let i = s.trailing_zeros() as usize;
            s &= s - 1;
            let v = col[i];
            match out.iter().position(|&(cs, _)| col[cs.trailing_zeros() as usize] == v) {
                Some(j) => {
                    out[j].0 |= 1 << i;
                    out[j].1 += self.w[i];
                }
                None => out.push((1 << i, self.w[i])),
            }
        }
    }

    /// Exact max probability of solving with the next challenge after using `k` hints first.
    fn pbar1(&mut self, set: u64, known: u8, k: u8) -> f64 {
        if let Some(&p) = self.pbar_memo.get(&(set, known, k)) {
            return p;
        }
        let unknown: Vec<usize> = (0..6).filter(|&f| known & (1 << f) == 0).collect();
        let total = self.mass(set) as f64;
        let k = (k as usize).min(unknown.len());
        let mut sum = 0.0;
        let mut subsets = 0u32;
        let limit = 1u32 << unknown.len();
        for sub in 0..limit {
            if sub.count_ones() as usize != k {
                continue;
            }
            subsets += 1;
            // Group candidates by their shown values on the hinted fields.
            let mut keyed: Vec<(u64, usize)> = Vec::with_capacity(64);
            let mut s = set;
            while s != 0 {
                let i = s.trailing_zeros() as usize;
                s &= s - 1;
                let mut key = 0u64;
                for (bit, &f) in unknown.iter().enumerate() {
                    if sub & (1 << bit) != 0 {
                        key = key << 8 | self.shown[f][i] as u64;
                    }
                }
                keyed.push((key, i));
            }
            keyed.sort_unstable();
            let mut best_total = 0u64;
            let mut j = 0;
            while j < keyed.len() {
                let key = keyed[j].0;
                let mut touched: Vec<u16> = Vec::new();
                let mut best = 0u64;
                while j < keyed.len() && keyed[j].0 == key {
                    let i = keyed[j].1;
                    for &s in &self.solvers[i] {
                        if self.acc[s as usize] == 0 {
                            touched.push(s);
                        }
                        self.acc[s as usize] += self.w[i];
                        best = best.max(self.acc[s as usize]);
                    }
                    j += 1;
                }
                for s in touched {
                    self.acc[s as usize] = 0;
                }
                best_total += best;
            }
            sum += best_total as f64 / total;
        }
        let p = sum / subsets as f64;
        self.pbar_memo.insert((set, known, k as u8), p);
        p
    }

    /// Admissible upper bound of a node's value (see module docs).
    fn upper(&mut self, n: LNode) -> Val {
        if n.challenges == 0 || n.set == 0 {
            return Val::ZERO;
        }
        let p = self.pbar1(n.set, n.known, n.hints);
        let c = n.challenges;
        let g = |j: u8| -> Val { if j > c { Val::ZERO } else { self.cont.value(n.hints, c - j) } };
        let (g1, g2) = (g(1), g(2));
        let s = if c == 1 { p * (1.0 + g1.0[0]) } else { p * (g1.0[0] - g2.0[0]) + 1.0 + g2.0[0] };
        let m_cont = if c == 1 { p * g1.0[1] } else { p * (g1.0[1] - g2.0[1]) + g2.0[1] };
        let m = (6 - n.matched.count_ones()) as f64 + m_cont;
        let a = if c >= 2 { 2.0 - self.pbar1(n.set, n.known, 0) } else { 1.0 };
        Val::new(s, m, -a)
    }

    fn visit(&mut self, n: LNode, parent_actions: &[u16]) -> Result<Val, OutOfBudget> {
        if n.challenges == 0 || n.set == 0 {
            return Ok(Val::ZERO);
        }
        let key = n.key();
        if let Some(&v) = self.memo.get(&key) {
            return Ok(v);
        }
        self.expanded += 1;
        if self.expanded > self.budget {
            return Err(OutOfBudget);
        }
        let v = self.expand(n, parent_actions, None)?;
        self.memo.insert(key, v);
        Ok(v)
    }

    /// Distinct, non-no-op challenges at `n` with their cells, ordered for branch-and-bound
    /// (solve probability first, then more cells).
    fn challenges_at(&self, n: LNode, parent_actions: &[u16]) -> Vec<(u16, Vec<LCell>)> {
        let mut out: Vec<(u16, Vec<LCell>)> = Vec::with_capacity(parent_actions.len());
        let mut seen: FxHashMap<Vec<(u64, u8, bool)>, ()> = FxHashMap::default();
        let mut cells = Vec::new();
        for &a in parent_actions {
            self.partition(a, n.set, &mut cells);
            if cells.len() == 1 && !cells[0].solved && cells[0].mask & !n.matched == 0 {
                continue;
            }
            let sig: Vec<(u64, u8, bool)> =
                cells.iter().map(|c| (c.set, c.mask & !n.matched, c.solved)).collect();
            if seen.insert(sig, ()).is_some() {
                continue;
            }
            out.push((a, cells.clone()));
        }
        let solve_mass = |cs: &[LCell]| cs.iter().filter(|c| c.solved).map(|c| c.mass).sum::<u64>();
        out.sort_by(|x, y| {
            solve_mass(&y.1).cmp(&solve_mass(&x.1)).then(y.1.len().cmp(&x.1.len())).then(x.0.cmp(&y.0))
        });
        out
    }

    fn expand(
        &mut self,
        n: LNode,
        parent_actions: &[u16],
        mut record: Option<&mut Vec<RootQ>>,
    ) -> Result<Val, OutOfBudget> {
        let total = self.mass(n.set) as f64;
        let node_ub = if self.no_bounds { None } else { Some(self.upper(n)) };
        let chals = self.challenges_at(n, parent_actions);
        let child_actions: Vec<u16> = chals.iter().map(|(a, _)| *a).collect();
        let mut best: Option<Val> = None;
        let reached = |best: &Option<Val>| -> bool {
            matches!((best, node_ub), (Some(b), Some(u)) if b.cmp_eps(&u) != Ordering::Less)
        };

        // HINT.
        let unknown = !n.known & ALL_FIELDS;
        if n.hints > 0 && unknown != 0 {
            let fields: Vec<Field> = Field::ALL.into_iter().filter(|f| unknown & f.bit() != 0).collect();
            let mut children: Vec<(LNode, f64)> = Vec::new();
            let mut cells = Vec::new();
            for &f in &fields {
                self.hint_cells(f, n.set, &mut cells);
                for &(cs, m) in &cells {
                    let child =
                        self.normalize(LNode { set: cs, known: n.known | f.bit(), hints: n.hints - 1, ..n });
                    children.push((child, m as f64 / total / fields.len() as f64));
                }
            }
            let mut v = Val::ZERO;
            for &(child, p) in &children {
                v += self.visit(child, &child_actions)? * p;
            }
            let v = v + Val::new(0.0, 0.0, -1.0);
            if let Some(r) = record.as_deref_mut() {
                r.push(RootQ { action: Action::Hint, value: Some(v), upper: v });
            }
            best = Some(v);
        }

        // Challenges.
        for (a, cells) in &chals {
            let children: Vec<(Option<LNode>, Val, f64)> = cells
                .iter()
                .map(|c| {
                    let reward = challenge_reward(n.matched, c.mask, c.solved);
                    let p = c.mass as f64 / total;
                    if c.solved {
                        (None, reward + self.cont.value(n.hints, n.challenges - 1), p)
                    } else {
                        let child = self.normalize(LNode {
                            set: c.set,
                            known: n.known | c.mask,
                            matched: n.matched | c.mask,
                            hints: n.hints,
                            challenges: n.challenges - 1,
                        });
                        (Some(child), reward, p)
                    }
                })
                .collect();
            let guess = self.actions[*a as usize];
            // At the root (when recording) every action is evaluated exactly, so the UI can show
            // alternatives and the hint-vs-challenge difference; pruning still applies below.
            if !self.no_bounds
                && record.is_none()
                && let Some(b) = best
            {
                let mut ub = Val::ZERO;
                for &(child, r, p) in &children {
                    ub += (r + child.map_or(Val::ZERO, |c| self.upper(c))) * p;
                }
                if ub.cmp_eps(&b) == Ordering::Less {
                    self.pruned += 1;
                    if let Some(rec) = record.as_deref_mut() {
                        rec.push(RootQ { action: Action::Challenge(guess), value: None, upper: ub });
                    }
                    continue;
                }
            }
            let mut v = Val::ZERO;
            for &(child, r, p) in &children {
                let cv = match child {
                    Some(c) => self.visit(c, &child_actions)?,
                    None => Val::ZERO,
                };
                v += (r + cv) * p;
            }
            if let Some(rec) = record.as_deref_mut() {
                rec.push(RootQ { action: Action::Challenge(guess), value: Some(v), upper: v });
            }
            if best.is_none_or(|b| v.gt(&b)) {
                best = Some(v);
            }
            if record.is_none() && reached(&best) {
                break;
            }
        }
        Ok(best.unwrap_or(Val::ZERO))
    }

    pub fn memo_len(&self) -> usize {
        self.memo.len()
    }

    pub fn root_actions(&self) -> usize {
        self.actions.len()
    }

    pub fn solver_count(&self) -> usize {
        self.solver_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::belief::{Belief, Event};
    use crate::data::Pool;
    use crate::rng::Rng;
    use crate::rules::RulePolicy;
    use crate::search::reference;
    use crate::search::{LegacyContinuation, NoContinuation};
    use crate::synth::random_table;

    /// Exact solver (memo, reduction, no-op pruning, caps, branch-and-bound) equals brute force.
    #[test]
    fn matches_brute_force() {
        let mut rng = Rng::new(11);
        let mut checked = 0;
        for round in 0..150 {
            let policy =
                RulePolicy { unknown_stat_matches: round % 4 != 0, link_def_matches: round % 3 != 0 };
            let ctx = Ctx::new(random_table(&mut rng, 5 + (round % 5)), policy, Pool::MasterDuel);
            let all = ctx.pool_groups.clone();
            let target = all[rng.below(all.len() as u64) as usize];
            let mut b = Belief::prior(&ctx);
            if rng.below(2) == 0 {
                let f = Field::from_index(rng.below(6) as usize);
                b.apply(&ctx, &Event::Reveal { field: f, shown: ctx.table.shown(f, target) }).unwrap();
            }
            let hints = rng.below(3) as u8;
            let challenges = 1 + rng.below(3) as u8;
            let node = reference::node_of(&b, hints, challenges);
            let legacy = LegacyContinuation { remaining: 2 };
            let conts: [&dyn Continuation; 2] = [&NoContinuation, &legacy];
            for cont in conts {
                let want = reference::value(&ctx, &node, cont);
                for no_bounds in [false, true] {
                    let mut ex = ExactSolver::new(&ctx, cont, &b.cands, &all, u64::MAX).unwrap();
                    ex.no_bounds = no_bounds;
                    let got = ex.value(&node).unwrap();
                    assert_eq!(
                        got.cmp_eps(&want),
                        Ordering::Equal,
                        "round {round} bounds {}: {got:?} vs {want:?}",
                        !no_bounds
                    );
                    let mut ex = ExactSolver::new(&ctx, cont, &b.cands, &all, u64::MAX).unwrap();
                    ex.no_bounds = no_bounds;
                    let roots = ex.root(&node).unwrap();
                    let best = roots.iter().filter_map(|q| q.value).fold(None::<Val>, |a, v| match a {
                        Some(x) if !v.gt(&x) => Some(x),
                        _ => Some(v),
                    });
                    assert_eq!(best.unwrap_or(Val::ZERO).cmp_eps(&want), Ordering::Equal);
                    for q in &roots {
                        assert!(q.upper.cmp_eps(&q.value.unwrap_or(q.upper)) != Ordering::Less);
                    }
                }
                checked += 1;
            }
        }
        assert!(checked >= 250);
    }

    /// The node upper bound is admissible on every node of random searches.
    #[test]
    fn upper_bound_is_admissible() {
        let mut rng = Rng::new(5);
        for round in 0..80 {
            let ctx =
                Ctx::new(random_table(&mut rng, 4 + round % 6), RulePolicy::default(), Pool::MasterDuel);
            let all = ctx.pool_groups.clone();
            let b = Belief::prior(&ctx);
            let legacy = LegacyContinuation { remaining: 3 };
            let conts: [&dyn Continuation; 2] = [&NoContinuation, &legacy];
            for cont in conts {
                let (h, c) = (rng.below(3) as u8, 1 + rng.below(3) as u8);
                let node = reference::node_of(&b, h, c);
                let want = reference::value(&ctx, &node, cont);
                let mut ex = ExactSolver::new(&ctx, cont, &b.cands, &all, u64::MAX).unwrap();
                let root = ex.lnode(&node);
                let ub = ex.upper(root);
                for i in 0..3 {
                    assert!(
                        ub.0[i] >= want.0[i] - 1e-9,
                        "round {round} comp {i}: ub {ub:?} < value {want:?}"
                    );
                }
            }
        }
    }
}
