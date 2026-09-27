//! Belief tracking within one puzzle. The posterior over targets is exactly the prior weight
//! restricted to the consistent set: every observation is a deterministic function of the target,
//! and a hint's field choice does not depend on the target.

use crate::ctx::Ctx;
use crate::data::{Field, Gid};
use crate::rules::Fb;

/// What happened in a puzzle, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A field's value became known without a challenge (initial reveal or hint).
    Reveal { field: Field, shown: u8 },
    /// A challenge with `guess` was judged `fb`.
    Challenge { guess: Gid, fb: Fb },
    /// A field already matched by an earlier challenge that was not recorded (starting mid-event):
    /// filters like a reveal and counts as a challenge match.
    Matched { field: Field, shown: u8 },
}

/// Candidate set plus what is known about the target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Belief {
    /// Consistent targets, ascending.
    pub cands: Vec<Gid>,
    /// Fields whose value is known (from reveals, hints or challenge matches).
    pub known: u8,
    /// Fields matched by a challenge (the ones that count for match rewards).
    pub matched: u8,
    /// Known shown values per field (level: rule level).
    pub values: [Option<u8>; 6],
    /// Cards already challenged in this puzzle.
    pub guessed: Vec<Gid>,
    /// A challenge fully matched (the puzzle is over).
    pub solved: bool,
}

/// An event that leaves no consistent target (or contradicts what is known).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contradiction {
    pub event: usize,
}

/// Applying an event would leave no consistent target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inconsistent;

impl Belief {
    /// Before any event: every group of the pool.
    pub fn prior(ctx: &Ctx) -> Belief {
        Belief {
            cands: ctx.pool_groups.clone(),
            known: 0,
            matched: 0,
            values: [None; 6],
            guessed: Vec::new(),
            solved: false,
        }
    }

    pub fn from_events(ctx: &Ctx, events: &[Event]) -> Result<Belief, Contradiction> {
        let mut b = Belief::prior(ctx);
        for (i, ev) in events.iter().enumerate() {
            b.apply(ctx, ev).map_err(|_| Contradiction { event: i })?;
        }
        Ok(b)
    }

    /// Apply one event; on error the belief is left unchanged.
    pub fn apply(&mut self, ctx: &Ctx, ev: &Event) -> Result<(), Inconsistent> {
        let t = &ctx.table;
        match *ev {
            Event::Reveal { field, shown } => {
                if let Some(v) = self.values[field as usize] {
                    return if v == shown { Ok(()) } else { Err(Inconsistent) };
                }
                let next: Vec<Gid> =
                    self.cands.iter().copied().filter(|&g| t.shown(field, g) == shown).collect();
                if next.is_empty() {
                    return Err(Inconsistent);
                }
                self.cands = next;
                self.known |= field.bit();
                self.values[field as usize] = Some(shown);
            }
            Event::Matched { field, shown } => {
                if self.values[field as usize].is_some_and(|v| v != shown) {
                    return Err(Inconsistent);
                }
                let next: Vec<Gid> = self.cands.iter().copied().filter(|&g| t.shown(field, g) == shown).collect();
                if next.is_empty() {
                    return Err(Inconsistent);
                }
                self.cands = next;
                self.known |= field.bit();
                self.matched |= field.bit();
                self.values[field as usize] = Some(shown);
            }
            Event::Challenge { guess, fb } => {
                if self.solved {
                    return Err(Inconsistent);
                }
                let row = ctx.rules.row(t, guess);
                let next: Vec<Gid> = self
                    .cands
                    .iter()
                    .copied()
                    .filter(|&g| row.feedback_packed(&ctx.rules, g) == fb)
                    .collect();
                if next.is_empty() {
                    return Err(Inconsistent);
                }
                let mask = fb.mask();
                let mut values = self.values;
                for f in Field::ALL {
                    if mask & f.bit() == 0 {
                        continue;
                    }
                    let v = match f {
                        Field::Frame => fb.frame_reveal().unwrap(),
                        Field::Level => fb.level_reveal().unwrap(),
                        _ => t.id(f, guess),
                    };
                    if values[f as usize].is_some_and(|old| old != v) {
                        return Err(Inconsistent);
                    }
                    values[f as usize] = Some(v);
                }
                self.cands = next;
                self.values = values;
                self.known |= mask;
                self.matched |= mask;
                self.solved = fb.solved();
                if !self.guessed.contains(&guess) {
                    self.guessed.push(guess);
                }
            }
        }
        Ok(())
    }

    #[inline]
    pub fn mass(&self, ctx: &Ctx) -> u64 {
        ctx.mass(&self.cands)
    }

    /// Fields a hint can still reveal.
    #[inline]
    pub fn unknown(&self) -> u8 {
        !self.known & crate::data::ALL_FIELDS
    }
}

/// When the events contradict each other (usually a mis-entered feedback), rank the events by how
/// much candidate weight removing that single event would restore. Best explanation first.
pub fn relax1(ctx: &Ctx, events: &[Event]) -> Vec<(usize, u64)> {
    let mut out: Vec<(usize, u64)> = (0..events.len())
        .filter_map(|skip| {
            let rest: Vec<Event> =
                events.iter().enumerate().filter(|&(i, _)| i != skip).map(|(_, e)| *e).collect();
            Belief::from_events(ctx, &rest).ok().map(|b| (skip, b.mass(ctx)))
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::tests::tiny_groups;
    use crate::data::{CardTable, Pool};
    use crate::rules::RulePolicy;

    fn ctx() -> Ctx {
        Ctx::new(CardTable::from_raw(&tiny_groups()).unwrap(), RulePolicy::default(), Pool::MasterDuel)
    }

    #[test]
    fn challenge_filters_and_records() {
        let ctx = ctx();
        let (zarc, exodia) = (0, 1);
        let fb = ctx.rules.feedback(&ctx.table, exodia, zarc);
        let b = Belief::from_events(&ctx, &[Event::Challenge { guess: exodia, fb }]).unwrap();
        assert!(b.cands.contains(&zarc));
        assert!(b.cands.iter().all(|&t| ctx.rules.feedback(&ctx.table, exodia, t) == fb));
        assert_eq!(b.matched, fb.mask());
        assert_eq!(b.values[Field::Frame as usize], Some(ctx.table.frame[zarc as usize]));
    }

    #[test]
    fn contradiction_is_reported_and_explained() {
        let ctx = ctx();
        let zarc = 0;
        let good = Event::Reveal { field: Field::Level, shown: 12 };
        let wrong = Event::Reveal { field: Field::Atk, shown: ctx.table.atk[6] }; // Link's ATK: no level-12 card has it
        let fb = ctx.rules.feedback(&ctx.table, zarc, zarc);
        let events = [good, wrong, Event::Challenge { guess: zarc, fb }];
        let err = Belief::from_events(&ctx, &events).unwrap_err();
        assert_eq!(err.event, 1);
        let ranked = relax1(&ctx, &events);
        assert_eq!(ranked[0].0, 1, "dropping the wrong reveal explains the contradiction");
    }

    #[test]
    fn matched_entry_filters_and_counts_as_match() {
        let ctx = ctx();
        let b = Belief::from_events(&ctx, &[Event::Matched { field: Field::Level, shown: 12 }]).unwrap();
        assert!(b.cands.iter().all(|&g| ctx.table.shown(Field::Level, g) == 12));
        assert_eq!(b.matched, Field::Level.bit());
        assert_eq!(b.known, Field::Level.bit());
    }
}
