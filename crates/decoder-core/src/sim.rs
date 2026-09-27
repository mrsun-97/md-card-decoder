//! Event simulation with common random numbers: the target and initial reveal of puzzle `p` of
//! activity `i` come from stream `(seed, i, p, 0)` and hint draws from `(seed, i, p, 1)`, so every
//! policy faces the same targets and paired comparisons have low variance.

use crate::belief::{Belief, Event};
use crate::ctx::Ctx;
use crate::data::{ALL_FIELDS, Field};
use crate::policy::{DecisionInput, Models, Planner, PolicyCfg};
use crate::rng::{Rng, mix};
use crate::search::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventCfg {
    pub puzzles: u8,
    pub hints: u8,
    pub challenges: u8,
}

/// One decision seen while playing, for fitting the leaf model.
#[derive(Clone, Copy, Debug)]
pub struct DecisionSample {
    pub entropy: f64,
    pub hints: u8,
    pub challenges: u8,
}

#[derive(Clone, Debug, Default)]
pub struct PuzzleOutcome {
    pub solved: bool,
    pub hints_used: u8,
    pub challenges_used: u8,
    /// Fields matched by challenges (6 when solved).
    pub matches: u8,
    /// (decision, hints used from there on, challenges used from there on).
    pub decisions: Vec<(DecisionSample, u8, u8)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActivityOutcome {
    pub solved: u8,
    pub matches: u32,
    pub hints_used: u32,
    pub challenges_used: u32,
}

/// Play one puzzle from its random target to a solve or until challenges run out.
#[allow(clippy::too_many_arguments)]
pub fn play_puzzle(
    ctx: &Ctx,
    models: &Models,
    planner: &mut Planner,
    cfg: &PolicyCfg,
    target_seed: u64,
    hint_seed: u64,
    remaining_after: u8,
    mut hints: u8,
    mut challenges: u8,
    record: bool,
) -> PuzzleOutcome {
    let mut trng = Rng::new(target_seed);
    let mut hrng = Rng::new(hint_seed);
    let weights: Vec<u32> = ctx.pool_groups.iter().map(|&g| ctx.weights[g as usize]).collect();
    let target = ctx.pool_groups[trng.weighted(&weights)];
    let field = Field::from_index(trng.below(6) as usize);
    let mut b = Belief::prior(ctx);
    b.apply(ctx, &Event::Reveal { field, shown: ctx.table.shown(field, target) })
        .expect("target is consistent");
    let (h0, c0) = (hints, challenges);
    let mut decisions: Vec<(DecisionSample, u8, u8)> = Vec::new();
    for _ in 0..64 {
        if b.solved || challenges == 0 {
            break;
        }
        let input = DecisionInput { belief: &b, hints, challenges, remaining_after };
        let Some(d) = planner.decide(ctx, models, cfg, input) else { break };
        if record {
            decisions.push((
                DecisionSample { entropy: ctx.entropy(&b.cands), hints, challenges },
                hints,
                challenges,
            ));
        }
        match d.action {
            Action::Hint => {
                let unknown: Vec<Field> =
                    Field::ALL.into_iter().filter(|f| (!b.known & ALL_FIELDS) & f.bit() != 0).collect();
                let f = unknown[hrng.below(unknown.len() as u64) as usize];
                b.apply(ctx, &Event::Reveal { field: f, shown: ctx.table.shown(f, target) }).unwrap();
                hints -= 1;
            }
            Action::Challenge(g) => {
                let fb = ctx.rules.feedback(&ctx.table, g, target);
                b.apply(ctx, &Event::Challenge { guess: g, fb }).unwrap();
                challenges -= 1;
            }
        }
    }
    // Convert (hints, challenges) at each decision into the amounts still used from there on.
    for d in &mut decisions {
        d.1 -= hints;
        d.2 -= challenges;
    }
    PuzzleOutcome {
        solved: b.solved,
        hints_used: h0 - hints,
        challenges_used: c0 - challenges,
        matches: b.matched.count_ones() as u8,
        decisions,
    }
}

/// Play a whole event (puzzles in order, shared inventory; running out of challenges ends it).
/// Decision samples of solved puzzles are appended to `samples` as (entropy, hints>0, J, E).
#[allow(clippy::too_many_arguments)]
pub fn play_activity(
    ctx: &Ctx,
    models: &Models,
    planner: &mut Planner,
    cfg: &PolicyCfg,
    event: EventCfg,
    seed: u64,
    index: u64,
    mut samples: Option<&mut Vec<(f64, bool, u8, u8)>>,
) -> ActivityOutcome {
    let (mut hints, mut challenges) = (event.hints, event.challenges);
    let mut out = ActivityOutcome::default();
    for p in 0..event.puzzles {
        let po = play_puzzle(
            ctx,
            models,
            planner,
            cfg,
            mix(&[seed, index, p as u64, 0]),
            mix(&[seed, index, p as u64, 1]),
            event.puzzles - p - 1,
            hints,
            challenges,
            samples.is_some(),
        );
        hints -= po.hints_used;
        challenges -= po.challenges_used;
        out.hints_used += po.hints_used as u32;
        out.challenges_used += po.challenges_used as u32;
        out.matches += po.matches as u32;
        if let Some(s) = samples.as_deref_mut()
            && po.solved
        {
            for (d, e, j) in &po.decisions {
                s.push((d.entropy, d.hints > 0, *j, *e));
            }
        }
        if !po.solved {
            break;
        }
        out.solved += 1;
    }
    out
}
