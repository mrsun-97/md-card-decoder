//! Exact action reduction: two guesses are interchangeable at a node when they induce the same
//! partition of the candidates with the same match rewards. That is decided in O(1) per guess from
//! a projected signature — per field, the set of *present* target values the guess matches, plus
//! the guess's full matches inside the candidate set (which decide the solved cell).
//!
//! Fields already matched by a challenge are dropped for frame, attribute, race, ATK and DEF: all
//! candidates share that value, so the response is constant and earns nothing. The level field is
//! never dropped: candidates sharing a rule level can still differ in their printed level (rule-level
//! cards), so a guess's level response can split them.

use crate::ctx::Ctx;
use crate::data::Gid;
use crate::metrics::{FieldHist, Marks};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Sig {
    frame: u16,
    level: u16,
    attribute: u8,
    race: u8,
    atk: u16,
    def: u16,
    fm: [u16; 3],
}

const MISS: u16 = u16::MAX;
const DROPPED: u16 = u16::MAX - 1;

fn signature(ctx: &Ctx, h: &FieldHist, in_c: &Marks, matched: u8, g: Gid) -> Sig {
    let t = &ctx.table;
    let gu = g as usize;
    let mut s = Sig { frame: 0, level: 0, attribute: 0, race: 0, atk: 0, def: 0, fm: [MISS; 3] };
    if matched & 1 == 0 {
        for (tf, &m) in h.frame.iter().enumerate() {
            if m > 0 && ctx.rules.frame_code(t.frame[gu], tf as u8) != 0 {
                s.frame |= 1 << tf;
            }
        }
    }
    for (tl, &m) in h.level.iter().enumerate() {
        if m > 0 && ctx.rules.level_code(t.level[gu], tl as u8) != 0 {
            s.level |= 1 << tl;
        }
    }
    let eq = |hist: &[u64], id: u8, strict: bool, bit: u8| -> u16 {
        if matched & bit != 0 {
            DROPPED
        } else if strict {
            // Only the guess's own group matches: the guess is its own class.
            if in_c.has(g) { 0x8000 | g } else { MISS }
        } else if hist[id as usize] > 0 {
            id as u16
        } else {
            MISS
        }
    };
    s.attribute = eq(&h.attribute, t.attribute[gu], false, 2) as u8;
    s.race = eq(&h.race, t.race[gu], false, 4) as u8;
    s.atk = eq(&h.atk, t.atk[gu], ctx.rules.atk_is_strict(t.atk[gu]), 16);
    s.def = eq(&h.def, t.def[gu], ctx.rules.def_is_strict(t.def[gu]), 32);
    let mut k = 0;
    for &x in ctx.full_matches(g) {
        if in_c.has(x) && k < 3 {
            s.fm[k] = x;
            k += 1;
        }
    }
    s.fm[..k].sort_unstable();
    s
}

/// One representative per equivalence class, in the order of `guesses` (the first member wins).
/// `in_c` must mark the candidate set that `h` describes.
pub fn representatives(ctx: &Ctx, h: &FieldHist, in_c: &Marks, matched: u8, guesses: &[Gid]) -> Vec<Gid> {
    let mut keyed: Vec<(Sig, u32)> =
        guesses.iter().enumerate().map(|(i, &g)| (signature(ctx, h, in_c, matched, g), i as u32)).collect();
    keyed.sort_unstable();
    let mut firsts: Vec<u32> = Vec::with_capacity(keyed.len());
    let mut prev: Option<Sig> = None;
    for (sig, i) in keyed {
        if prev != Some(sig) {
            firsts.push(i);
            prev = Some(sig);
        }
    }
    firsts.sort_unstable();
    firsts.into_iter().map(|i| guesses[i as usize]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Pool;
    use crate::kernel::Partitioner;
    use crate::rng::Rng;
    use crate::rules::RulePolicy;
    use crate::synth::random_table;

    /// Members of a class must produce identical cells (same members, same rewarded mask bits,
    /// same solved flag) — checked against every other guess, on random pools and subsets.
    #[test]
    fn classes_are_exact() {
        let mut rng = Rng::new(7);
        let mut part = Partitioner::default();
        for round in 0..300 {
            let policy =
                RulePolicy { unknown_stat_matches: round % 3 != 0, link_def_matches: round % 5 != 0 };
            let ctx = Ctx::new(random_table(&mut rng, 14), policy, Pool::MasterDuel);
            let all = ctx.pool_groups.clone();
            // A reachable state: a few random challenges against a random target.
            let target = all[rng.below(all.len() as u64) as usize];
            let mut belief = crate::belief::Belief::prior(&ctx);
            for _ in 0..rng.below(3) {
                let guess = all[rng.below(all.len() as u64) as usize];
                let fb = ctx.rules.feedback(&ctx.table, guess, target);
                if !fb.solved() {
                    belief.apply(&ctx, &crate::belief::Event::Challenge { guess, fb }).unwrap();
                }
            }
            let (cands, matched) = (belief.cands.clone(), belief.matched);
            let h = FieldHist::new(&ctx, &cands);
            let mut marks = Marks::default();
            marks.set(ctx.table.len(), &cands);
            let reps = representatives(&ctx, &h, &marks, matched, &all);
            let mut class_of = |g: Gid| -> Vec<(Vec<Gid>, u8, bool)> {
                let (mut cells, mut arena) = (Vec::new(), Vec::new());
                part.split(&ctx, g, &cands, &mut cells, &mut arena);
                let mut v: Vec<(Vec<Gid>, u8, bool)> = cells
                    .iter()
                    .map(|c| {
                        let members = arena[c.start as usize..(c.start + c.count) as usize].to_vec();
                        (members, c.fb.mask() & !matched, c.fb.solved())
                    })
                    .collect();
                v.sort();
                v
            };
            let rep_outcomes: Vec<_> = reps.iter().map(|&r| class_of(r)).collect();
            for &g in &all {
                let o = class_of(g);
                let hits = rep_outcomes.iter().filter(|r| **r == o).count();
                assert!(hits >= 1, "guess {g} matches no representative (round {round})");
            }
        }
    }
}
