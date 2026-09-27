//! The public recommendation pipeline: a decision plus everything the UI needs to explain it.

use crate::belief::Belief;
use crate::cont::LearnedLeaf;
use crate::ctx::Ctx;
use crate::data::Gid;
use crate::kernel::Partitioner;
use crate::metrics::{Marks, entropy, quick_all};
use crate::policy::{Decision, DecisionInput, Models, Planner, PolicyCfg};
use crate::search::Action;
use crate::value::Val;

/// Per-action numbers shown next to a recommendation.
#[derive(Clone, Debug)]
pub struct ActionStats {
    pub action: Action,
    /// Q-value from the search.
    pub value: Val,
    /// Probability that this challenge solves the puzzle now.
    pub p_solve: f64,
    /// Expected number of first-time matched fields.
    pub e_new_matches: f64,
    /// Feedback entropy in bits (information gained on average).
    pub entropy: f64,
    /// Number of distinct outcomes.
    pub outcomes: usize,
}

#[derive(Clone, Debug)]
pub struct Forecast {
    pub p_solve_puzzle: f64,
    pub e_challenges: f64,
    pub e_hints: f64,
}

#[derive(Clone, Debug)]
pub struct Recommendation {
    pub decision: Decision,
    /// The best action first, then alternatives (best first), at most `max_alternatives + 1`.
    pub actions: Vec<ActionStats>,
    /// Q-value of using a hint now, when one is available and was evaluated.
    pub hint_value: Option<Val>,
    /// Best challenge Q-value (to compare with the hint).
    pub best_challenge_value: Option<Val>,
    /// Actions tied with the best within tolerance ("任选其一").
    pub ties: Vec<Action>,
    pub candidates: usize,
    pub mass: u64,
    /// Posterior entropy of the candidate set, bits.
    pub entropy: f64,
    /// Most likely targets with their probabilities.
    pub top: Vec<(Gid, f64)>,
    pub forecast: Option<Forecast>,
}

pub fn recommend(
    ctx: &Ctx,
    models: &Models,
    planner: &mut Planner,
    cfg: &PolicyCfg,
    input: DecisionInput,
    max_alternatives: usize,
) -> Option<Recommendation> {
    let decision = planner.decide(ctx, models, cfg, input)?;
    let b: &Belief = input.belief;
    let total = ctx.mass(&b.cands);
    let mut marks = Marks::default();
    let (_, qs) = quick_all(ctx, &b.cands, &ctx.pool_groups, &mut marks);
    let quick_of = |g: Gid| qs.iter().find(|q| q.guess == g);
    let mut part = Partitioner::default();
    let mut cells = Vec::new();
    let mut stats = Vec::new();
    for &(action, value) in decision.q.iter().take(max_alternatives + 1) {
        let s = match action {
            Action::Hint => {
                ActionStats { action, value, p_solve: 0.0, e_new_matches: 0.0, entropy: 0.0, outcomes: 0 }
            }
            Action::Challenge(g) => {
                part.cells(ctx, g, &b.cands, &mut cells);
                let q = quick_of(g);
                ActionStats {
                    action,
                    value,
                    p_solve: q.map_or(0.0, |q| q.solve_mass as f64 / total as f64),
                    e_new_matches: q.map_or(0.0, |q| q.new_match_mass(b.matched) as f64 / total as f64),
                    entropy: entropy(&cells, total),
                    outcomes: cells.len(),
                }
            }
        };
        stats.push(s);
    }
    let hint_value = decision.q.iter().find(|(a, _)| *a == Action::Hint).map(|&(_, v)| v);
    let best_challenge_value =
        decision.q.iter().find(|(a, _)| matches!(a, Action::Challenge(_))).map(|&(_, v)| v);
    let ties = decision
        .q
        .iter()
        .filter(|(_, v)| v.cmp_eps(&decision.value) == std::cmp::Ordering::Equal)
        .map(|&(a, _)| a)
        .collect();
    let mut top: Vec<(Gid, f64)> =
        b.cands.iter().map(|&g| (g, ctx.weights[g as usize] as f64 / total as f64)).collect();
    top.sort_by(|a, x| x.1.total_cmp(&a.1).then(a.0.cmp(&x.0)));
    top.truncate(10);
    let h = ctx.entropy(&b.cands);
    let forecast = models.leaf.as_ref().map(|ctg| {
        let leaf = LearnedLeaf::new(ctg, &models.cont, input.remaining_after);
        let (p, ej, ee) = leaf.forecast(h, input.hints, input.challenges);
        Forecast { p_solve_puzzle: p, e_challenges: ej, e_hints: ee }
    });
    Some(Recommendation {
        decision,
        actions: stats,
        hint_value,
        best_challenge_value,
        ties,
        candidates: b.cands.len(),
        mass: total,
        entropy: h,
        top,
        forecast,
    })
}
