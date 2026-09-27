//! Decision policies: which action to take in a puzzle state (PLAN.md §4.5–4.6).
//!
//! - `PolicyCfg::legacy()` — a native port of the old tool's strict mode: legacy root action set,
//!   depth-2 search, zero horizon value, heuristic continuation. The baseline to beat.
//! - `PolicyCfg::fv2()` — the new main tier: same root set (plus optional top-K by entropy),
//!   depth-2 search, learned cost-to-go leaf, fitted continuation table F̂; exact tier when small.

use crate::actions::representatives;
use crate::belief::Belief;
use crate::cont::{ContTable, CostToGo, LearnedLeaf, TableCont, ZeroLeaf};
use crate::ctx::Ctx;
use crate::data::{Field, Gid};
use crate::kernel::Partitioner;
use crate::metrics::{FieldHist, Marks, Quick, quick_all, top_k_entropy};
use crate::search::exact::ExactSolver;
use crate::search::horizon::Horizon;
use crate::search::{Action, Continuation, LegacyContinuation, NoContinuation, Node};
use crate::value::Val;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafKind {
    Zero,
    Learned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContKind {
    Legacy,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolicyCfg {
    pub depth: u8,
    pub leaf: LeafKind,
    pub cont: ContKind,
    /// Extra root actions: the top-K guesses by exact feedback entropy.
    pub extra_entropy: usize,
    /// Try the exact tier when the candidate set is small.
    pub exact: bool,
    pub exact_budget: u64,
    /// Largest candidate set for the exact tier on the last puzzle.
    pub exact_max_n: usize,
    /// Largest candidate set for the exact tier on other puzzles (uses F̂ as continuation).
    pub exact_nonfinal_max_n: usize,
    /// Only consider guesses consistent with the known fields (optional UI setting).
    pub consistent_only: bool,
    /// Practical tolerance per objective component when choosing among root actions: options
    /// within `tol[0]` expected puzzles count as equal on that component and the choice falls
    /// through to matches, then fewer actions. Search estimates are far coarser than 1e-3, so
    /// strict comparison would pick on noise. Not used for the exact last-puzzle tier.
    pub root_tolerance: [f64; 3],
}

impl PolicyCfg {
    /// Native port of the legacy strict policy.
    pub fn legacy() -> PolicyCfg {
        PolicyCfg {
            depth: 2,
            leaf: LeafKind::Zero,
            cont: ContKind::Legacy,
            extra_entropy: 0,
            exact: false,
            exact_budget: 0,
            exact_max_n: 0,
            exact_nonfinal_max_n: 0,
            consistent_only: false,
            root_tolerance: [crate::value::EPS; 3],
        }
    }

    /// Depth-2 search with the fitted continuation and a zero horizon value (used to fit F̂).
    pub fn fitted_zero_leaf() -> PolicyCfg {
        PolicyCfg { cont: ContKind::Table, ..PolicyCfg::legacy() }
    }

    /// The new main policy: learned leaf + fitted continuation, exact tier when small.
    pub fn fv2() -> PolicyCfg {
        PolicyCfg {
            leaf: LeafKind::Learned,
            cont: ContKind::Table,
            exact: true,
            exact_budget: 30_000,
            exact_max_n: 16,
            exact_nonfinal_max_n: 7,
            root_tolerance: [1e-4, 1e-3, crate::value::EPS],
            ..PolicyCfg::legacy()
        }
    }
}

/// Fitted models the policies consult.
#[derive(Clone, Debug)]
pub struct Models {
    pub cont: ContTable,
    pub leaf: Option<CostToGo>,
}

impl Models {
    pub fn legacy_only() -> Models {
        Models { cont: ContTable::legacy(15, 20, 60), leaf: None }
    }
}

/// What a policy decides from.
#[derive(Clone, Copy, Debug)]
pub struct DecisionInput<'a> {
    pub belief: &'a Belief,
    pub hints: u8,
    pub challenges: u8,
    /// Puzzles still to come after this one.
    pub remaining_after: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Exact optimum of the last puzzle.
    ExactFinal,
    /// Exact within the puzzle, relative to the continuation table.
    ExactInPuzzle,
    /// Depth-limited search.
    Horizon { depth: u8 },
}

#[derive(Clone, Debug)]
pub struct Decision {
    pub action: Action,
    pub value: Val,
    pub method: Method,
    /// Q-values of the evaluated root actions, best first.
    pub q: Vec<(Action, Val)>,
    pub root_actions: usize,
}

/// Reusable scratch space for decisions (one per thread).
#[derive(Default)]
pub struct Planner {
    part: Partitioner,
    marks: Marks,
}

impl Planner {
    /// Decide the next action, or `None` when no challenge is left.
    pub fn decide(
        &mut self,
        ctx: &Ctx,
        models: &Models,
        cfg: &PolicyCfg,
        input: DecisionInput,
    ) -> Option<Decision> {
        let b = input.belief;
        if input.challenges == 0 || b.cands.is_empty() || b.solved {
            return None;
        }
        let node = Node {
            cands: b.cands.clone(),
            known: b.known,
            matched: b.matched,
            hints: input.hints,
            challenges: input.challenges,
        };
        let universe = self.universe(ctx, b, cfg);
        let k = input.remaining_after;
        let table_cont = TableCont { table: &models.cont, k };
        let legacy_cont = LegacyContinuation { remaining: k };
        let cont: &dyn Continuation = match cfg.cont {
            ContKind::Legacy => &legacy_cont,
            ContKind::Table => &table_cont,
        };
        if cfg.exact {
            let n = b.cands.len();
            let tier = if k == 0 && n <= cfg.exact_max_n {
                Some((&NoContinuation as &dyn Continuation, Method::ExactFinal))
            } else if n <= cfg.exact_nonfinal_max_n {
                Some((cont, Method::ExactInPuzzle))
            } else {
                None
            };
            if let Some((c, method)) = tier
                && let Ok(mut ex) = ExactSolver::new(ctx, c, &b.cands, &universe, cfg.exact_budget)
                && let Ok(roots) = ex.root(&node)
            {
                let q: Vec<(Action, Val)> =
                    roots.iter().filter_map(|r| r.value.map(|v| (r.action, v))).collect();
                let tol = if method == Method::ExactFinal { [crate::value::EPS; 3] } else { cfg.root_tolerance };
                if let Some(d) = pick(q, method, ex.root_actions(), tol) {
                    return Some(d);
                }
            }
        }
        let actions = self.root_actions(ctx, b, &universe, cfg);
        let root_actions = actions.len();
        let learned;
        let leaf: &dyn crate::cont::Leaf = match (cfg.leaf, &models.leaf) {
            (LeafKind::Learned, Some(ctg)) => {
                learned = LearnedLeaf::new(ctg, &models.cont, k);
                &learned
            }
            _ => &ZeroLeaf,
        };
        let mut hz = Horizon::new(ctx, cont, leaf, actions);
        let q = hz.root(&node, cfg.depth);
        pick(q, Method::Horizon { depth: cfg.depth }, root_actions, cfg.root_tolerance)
    }

    fn universe(&self, ctx: &Ctx, b: &Belief, cfg: &PolicyCfg) -> Vec<Gid> {
        if !cfg.consistent_only {
            return ctx.pool_groups.clone();
        }
        ctx.pool_groups
            .iter()
            .copied()
            .filter(|&g| {
                Field::ALL.iter().all(|&f| b.values[f as usize].is_none_or(|v| ctx.table.shown(f, g) == v))
            })
            .collect()
    }

    /// Legacy-style root set: 18 best by (solve, new matches), 8 by approximate information,
    /// 8 heaviest candidates, plus the top-K by exact entropy; one per equivalence class.
    pub fn root_actions(&mut self, ctx: &Ctx, b: &Belief, universe: &[Gid], cfg: &PolicyCfg) -> Vec<Gid> {
        let (h, qs) = quick_all(ctx, &b.cands, universe, &mut self.marks);
        let mut chosen: Vec<Gid> = Vec::with_capacity(40 + cfg.extra_entropy);
        let push = |list: &mut Vec<Gid>, g: Gid| {
            if !list.contains(&g) {
                list.push(g);
            }
        };
        let mut by_solve: Vec<&Quick> = qs.iter().collect();
        by_solve.sort_by(|a, x| {
            x.solve_mass
                .cmp(&a.solve_mass)
                .then(x.new_match_mass(b.matched).cmp(&a.new_match_mass(b.matched)))
                .then(a.guess.cmp(&x.guess))
        });
        by_solve.iter().take(18).for_each(|q| push(&mut chosen, q.guess));
        let info = |q: &Quick| -> f64 {
            (0..6).map(|f| binary_entropy(q.match_mass[f] as f64 / h.total as f64)).sum()
        };
        let mut by_info: Vec<(f64, Gid)> = qs.iter().map(|q| (info(q), q.guess)).collect();
        by_info.sort_by(|a, x| x.0.total_cmp(&a.0).then(a.1.cmp(&x.1)));
        by_info.iter().take(8).for_each(|&(_, g)| push(&mut chosen, g));
        let mut heavy: Vec<Gid> = b.cands.clone();
        heavy.sort_by(|a, x| ctx.weights[*x as usize].cmp(&ctx.weights[*a as usize]).then(a.cmp(x)));
        heavy.iter().take(8).for_each(|&g| push(&mut chosen, g));
        if cfg.extra_entropy > 0 {
            for (g, _) in top_k_entropy(ctx, &b.cands, &qs, cfg.extra_entropy, &mut self.part) {
                push(&mut chosen, g);
            }
        }
        dedup(ctx, &h, &mut self.marks, &b.cands, b.matched, &chosen)
    }
}

fn dedup(ctx: &Ctx, h: &FieldHist, marks: &mut Marks, cands: &[Gid], matched: u8, list: &[Gid]) -> Vec<Gid> {
    marks.set(ctx.table.len(), cands);
    representatives(ctx, h, marks, matched, list)
}

fn binary_entropy(p: f64) -> f64 {
    if p <= 0.0 || p >= 1.0 { 0.0 } else { -p * libm::log2(p) - (1.0 - p) * libm::log2(1.0 - p) }
}

/// Best action (first maximum in evaluation order), with the Q list sorted best first.
fn pick(mut q: Vec<(Action, Val)>, method: Method, root_actions: usize, tol: [f64; 3]) -> Option<Decision> {
    if q.is_empty() {
        return None;
    }
    // Lexicographic choice with a tolerance per component: keep the options within tol[i] of the
    // best on component i, then move to the next component. First in evaluation order wins ties.
    let mut pool: Vec<usize> = (0..q.len()).collect();
    for (i, &t) in tol.iter().enumerate() {
        let best = pool.iter().map(|&k| q[k].1.0[i]).fold(f64::NEG_INFINITY, f64::max);
        pool.retain(|&k| q[k].1.0[i] >= best - t);
    }
    let (action, value) = q[pool[0]];
    // Display order: chosen action first, then an exact lexicographic total order (the tolerance
    // comparison is not transitive and must never be used as a sort comparator).
    q.sort_by(|a, b| (b.0 == action).cmp(&(a.0 == action)).then(b.1.total_cmp(&a.1)));
    Some(Decision { action, value, method, q, root_actions })
}
