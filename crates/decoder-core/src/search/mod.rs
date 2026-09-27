//! Solvers. See PLAN.md §4.
//!
//! A puzzle node is (candidate set, known fields, matched fields, hints, challenges). The guessed
//! set is *not* part of the state: re-guessing a card is a strictly dominated no-op (all remaining
//! candidates share its feedback and its matches are already counted).

pub mod exact;
pub mod horizon;
pub mod reference;

use crate::data::Gid;
use crate::value::Val;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Hint,
    Challenge(Gid),
}

/// Value of the rest of the event after the current puzzle is solved with (hints, challenges)
/// left. Components: (expected further puzzles solved, expected further matched fields, 0).
pub trait Continuation {
    fn value(&self, hints: u8, challenges: u8) -> Val;
    /// True when the continuation is identically zero (last puzzle): enables resource caps.
    fn is_zero(&self) -> bool {
        false
    }
}

/// The last puzzle: nothing follows.
pub struct NoContinuation;

impl Continuation for NoContinuation {
    fn value(&self, _: u8, _: u8) -> Val {
        Val::ZERO
    }
    fn is_zero(&self) -> bool {
        true
    }
}

/// The legacy heuristic `min(r, c, (c + 0.61h) / 3.9)` solved puzzles (× 6 matches), kept as a
/// baseline until fitted tables exist.
pub struct LegacyContinuation {
    pub remaining: u8,
}

impl Continuation for LegacyContinuation {
    fn value(&self, hints: u8, challenges: u8) -> Val {
        if self.remaining == 0 || challenges == 0 {
            return Val::ZERO;
        }
        let equiv = challenges as f64 + 0.61 * hints as f64;
        let s = (self.remaining as f64).min(challenges as f64).min(equiv / 3.9);
        Val::new(s, 6.0 * s, 0.0)
    }
}

/// A puzzle node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub cands: Vec<Gid>,
    pub known: u8,
    pub matched: u8,
    pub hints: u8,
    pub challenges: u8,
}

/// Reward of a challenge outcome: (solved, first-time matched fields, −1 action).
#[inline]
pub fn challenge_reward(matched: u8, mask: u8, solved: bool) -> Val {
    let gain = (matched | mask).count_ones() - matched.count_ones();
    Val::new(solved as u8 as f64, gain as f64, -1.0)
}
