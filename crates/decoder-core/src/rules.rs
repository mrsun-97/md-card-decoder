//! Game rules: how a challenge is judged and what it reveals.
//!
//! Confirmed by official examples / the user (see PLAN.md §3.2):
//! - frame matches iff the frame bits intersect (a pendulum frame matches either of its parts, on
//!   the guess side as well); a frame match reveals the target's full frame;
//! - level/rank/link is one number; each card matches on its rule level *and* its printed level
//!   (both sides); a level match reveals the target's rule level;
//! - attribute, race, ATK, DEF match by equality.
//!
//! Unconfirmed (switchable): whether two *different* cards with '?' ATK/DEF match each other (R9)
//! and whether two different Link monsters match on DEF (R10). Identical groups always match, since
//! a card must be able to match itself.

use crate::data::{CardTable, Field, Gid, STAT_NONE, STAT_UNKNOWN};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RulePolicy {
    /// R9: '?' ATK (or DEF) of two different cards counts as a match.
    pub unknown_stat_matches: bool,
    /// R10: the (non-existent) DEF of two different Link monsters counts as a match.
    pub link_def_matches: bool,
}

impl Default for RulePolicy {
    fn default() -> Self {
        RulePolicy { unknown_stat_matches: true, link_def_matches: true }
    }
}

/// Feedback of one challenge, packed into 12 bits:
/// `frame_code << 8 | level_code << 4 | eq`, where `frame_code` is 0 (no match) or 1 + the
/// target's frame id, `level_code` is 0 or the target's rule level, and `eq` holds the
/// attribute/race/ATK/DEF match bits (bits 0..3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fb(pub u16);

/// Number of distinct feedback codes (histogram size).
pub const FB_CODES: usize = 1 << 12;

impl Fb {
    #[inline]
    pub fn frame_code(self) -> u8 {
        (self.0 >> 8) as u8
    }

    #[inline]
    pub fn level_code(self) -> u8 {
        ((self.0 >> 4) & 15) as u8
    }

    /// Match mask in field-bit order (frame bit 0 … DEF bit 5).
    #[inline]
    pub fn mask(self) -> u8 {
        let eq = (self.0 & 15) as u8;
        ((self.frame_code() != 0) as u8)
            | (eq & 1) << 1
            | (eq & 2) << 1
            | ((self.level_code() != 0) as u8) << 3
            | (eq & 4) << 2
            | (eq & 8) << 2
    }

    #[inline]
    pub fn solved(self) -> bool {
        self.frame_code() != 0 && self.level_code() != 0 && self.0 & 15 == 15
    }

    /// Target frame id revealed by a frame match.
    #[inline]
    pub fn frame_reveal(self) -> Option<u8> {
        self.frame_code().checked_sub(1)
    }

    /// Target rule level revealed by a level match.
    #[inline]
    pub fn level_reveal(self) -> Option<u8> {
        let l = self.level_code();
        (l != 0).then_some(l)
    }

    /// Build a feedback code from its observable parts (what the user reads in the game).
    pub fn from_parts(mask: u8, frame_reveal: Option<u8>, level_reveal: Option<u8>) -> Option<Fb> {
        let fc = if mask & 1 != 0 { frame_reveal? as u16 + 1 } else { 0 };
        let lc = if mask & 8 != 0 { level_reveal? as u16 } else { 0 };
        if fc > 15 || lc > 15 {
            return None;
        }
        let eq = ((mask >> 1) & 1) | ((mask >> 1) & 2) | ((mask >> 2) & 4) | ((mask >> 2) & 8);
        Some(Fb(fc << 8 | lc << 4 | eq as u16))
    }
}

/// Precomputed rule tables for one table + policy. Cheap to build; no allocation in the hot loop.
#[derive(Clone, Debug)]
pub struct Rules {
    pub policy: RulePolicy,
    /// `[guess frame id][target frame id]` → frame code.
    frame_tab: [[u8; 16]; 16],
    /// `[guess level id][target level id]` → level code.
    level_tab: [[u8; 16]; 16],
    /// ATK ids whose equality only counts within the same group.
    atk_strict: [bool; 256],
    /// DEF ids whose equality only counts within the same group.
    def_strict: [bool; 256],
    /// Row-major copy of the judged columns for one-load access in hot loops:
    /// `[frame, level, attribute, race, atk, def, 0, 0]`.
    packed: Vec<[u8; 8]>,
}

impl Rules {
    pub fn new(table: &CardTable, policy: RulePolicy) -> Rules {
        let d = &table.dicts;
        let mut frame_tab = [[0u8; 16]; 16];
        for (gi, &gb) in d.frame.iter().enumerate() {
            for (ti, &tb) in d.frame.iter().enumerate() {
                if gb & tb != 0 {
                    frame_tab[gi][ti] = ti as u8 + 1;
                }
            }
        }
        let mut level_tab = [[0u8; 16]; 16];
        for (gi, &gnm) in d.level_nm.iter().enumerate() {
            for (ti, &tnm) in d.level_nm.iter().enumerate() {
                if gnm & tnm != 0 {
                    level_tab[gi][ti] = table.level_n_of_id(ti as u8);
                }
            }
        }
        let mut atk_strict = [false; 256];
        for (i, &v) in d.atk.iter().enumerate() {
            atk_strict[i] = v == STAT_UNKNOWN && !policy.unknown_stat_matches;
        }
        let mut def_strict = [false; 256];
        for (i, &v) in d.def.iter().enumerate() {
            def_strict[i] = (v == STAT_UNKNOWN && !policy.unknown_stat_matches)
                || (v == STAT_NONE && !policy.link_def_matches);
        }
        let packed = (0..table.len())
            .map(|g| {
                [
                    table.frame[g],
                    table.level[g],
                    table.attribute[g],
                    table.race[g],
                    table.atk[g],
                    table.def[g],
                    0,
                    0,
                ]
            })
            .collect();
        Rules { policy, frame_tab, level_tab, atk_strict, def_strict, packed }
    }

    /// Frame code for (guess frame id, target frame id): 0 or 1 + target frame id.
    #[inline]
    pub fn frame_code(&self, guess_frame: u8, target_frame: u8) -> u8 {
        self.frame_tab[guess_frame as usize & 15][target_frame as usize & 15]
    }

    /// Level code for (guess level id, target level id): 0 or the target's rule level.
    #[inline]
    pub fn level_code(&self, guess_level: u8, target_level: u8) -> u8 {
        self.level_tab[guess_level as usize & 15][target_level as usize & 15]
    }

    /// ATK value ids whose equality only counts within the same group (policy R9).
    #[inline]
    pub fn atk_is_strict(&self, atk: u8) -> bool {
        self.atk_strict[atk as usize]
    }

    /// DEF value ids whose equality only counts within the same group (policy R9/R10).
    #[inline]
    pub fn def_is_strict(&self, def: u8) -> bool {
        self.def_strict[def as usize]
    }

    /// Judge guess `g` against target `t`.
    #[inline]
    pub fn feedback(&self, table: &CardTable, g: Gid, t: Gid) -> Fb {
        self.row(table, g).feedback(table, t)
    }

    /// Per-guess view with the table rows resolved, for tight loops over targets.
    #[inline]
    pub fn row<'a>(&'a self, table: &CardTable, g: Gid) -> GuessRow<'a> {
        let gu = g as usize;
        let atk = table.atk[gu];
        let def = table.def[gu];
        GuessRow {
            g,
            frame: &self.frame_tab[table.frame[gu] as usize],
            level: &self.level_tab[table.level[gu] as usize],
            attribute: table.attribute[gu],
            race: table.race[gu],
            atk,
            def,
            atk_strict: self.atk_strict[atk as usize],
            def_strict: self.def_strict[def as usize],
        }
    }
}

/// A guess with its rule-table rows resolved.
#[derive(Clone, Copy, Debug)]
pub struct GuessRow<'a> {
    pub g: Gid,
    frame: &'a [u8; 16],
    level: &'a [u8; 16],
    attribute: u8,
    race: u8,
    atk: u8,
    def: u8,
    atk_strict: bool,
    def_strict: bool,
}

impl GuessRow<'_> {
    /// Same as [`GuessRow::feedback`] but reads the target from the packed row copy.
    #[inline(always)]
    pub fn feedback_packed(&self, rules: &Rules, t: Gid) -> Fb {
        let p = rules.packed[t as usize];
        let fc = self.frame[(p[0] & 15) as usize] as u16;
        let lc = self.level[(p[1] & 15) as usize] as u16;
        let same = self.g == t;
        let a = (p[2] == self.attribute) as u16;
        let r = (p[3] == self.race) as u16;
        let atk = (p[4] == self.atk && (!self.atk_strict || same)) as u16;
        let def = (p[5] == self.def && (!self.def_strict || same)) as u16;
        Fb(fc << 8 | lc << 4 | a | r << 1 | atk << 2 | def << 3)
    }

    #[inline(always)]
    pub fn feedback(&self, table: &CardTable, t: Gid) -> Fb {
        let tu = t as usize;
        let fc = self.frame[table.frame[tu] as usize] as u16;
        let lc = self.level[table.level[tu] as usize] as u16;
        let same = self.g == t;
        let a = (table.attribute[tu] == self.attribute) as u16;
        let r = (table.race[tu] == self.race) as u16;
        let atk = (table.atk[tu] == self.atk && (!self.atk_strict || same)) as u16;
        let def = (table.def[tu] == self.def && (!self.def_strict || same)) as u16;
        Fb(fc << 8 | lc << 4 | a | r << 1 | atk << 2 | def << 3)
    }
}

/// Does group `g` agree with a shown value of `field`?
#[inline]
pub fn shows(table: &CardTable, g: Gid, field: Field, shown: u8) -> bool {
    table.shown(field, g) == shown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::tests::tiny_groups;

    fn tab() -> CardTable {
        CardTable::from_raw(&tiny_groups()).unwrap()
    }

    #[test]
    fn self_match_is_solved_under_every_policy() {
        let t = tab();
        for unknown_stat_matches in [false, true] {
            for link_def_matches in [false, true] {
                let rules = Rules::new(&t, RulePolicy { unknown_stat_matches, link_def_matches });
                for g in 0..t.len() as Gid {
                    let fb = rules.feedback(&t, g, g);
                    assert!(fb.solved(), "group {g} does not match itself");
                    assert_eq!(fb.mask(), 63);
                }
            }
        }
    }

    #[test]
    fn official_frame_examples() {
        let t = tab();
        let rules = Rules::new(&t, RulePolicy::default());
        let (zarc, exodia, clear_wing) = (0, 1, 2);
        // Example 1: target Zarc, guess the Fusion → frame matches, reveals Fusion/Pendulum.
        let fb = rules.feedback(&t, exodia, zarc);
        assert_eq!(fb.mask() & 1, 1);
        assert_eq!(t.dicts.frame[fb.frame_reveal().unwrap() as usize], 132);
        // Example 2: target Zarc, guess the Synchro/Pendulum → matches via pendulum.
        let fb = rules.feedback(&t, clear_wing, zarc);
        assert_eq!(t.dicts.frame[fb.frame_reveal().unwrap() as usize], 132);
        // Example 3: target the Fusion, guess Zarc → matches, reveals plain Fusion.
        let fb = rules.feedback(&t, zarc, exodia);
        assert_eq!(t.dicts.frame[fb.frame_reveal().unwrap() as usize], 4);
    }

    #[test]
    fn official_level_examples() {
        let t = tab();
        let rules = Rules::new(&t, RulePolicy::default());
        let (tzolkin, fno0, lv12) = (3, 4, 5);
        // Example 1: target Tzolkin, guess FNo.0 → printed 0 matches, reveals 12.
        assert_eq!(rules.feedback(&t, fno0, tzolkin).level_reveal(), Some(12));
        // Examples 2 and 3: Tzolkin ↔ level 12 → rule level matches, reveals 12.
        assert_eq!(rules.feedback(&t, lv12, tzolkin).level_reveal(), Some(12));
        assert_eq!(rules.feedback(&t, tzolkin, lv12).level_reveal(), Some(12));
        // Level 12 guess vs FNo.0 (rank 1 / printed 0) → no level match.
        assert_eq!(rules.feedback(&t, lv12, fno0).level_reveal(), None);
    }

    #[test]
    fn parts_roundtrip() {
        let t = tab();
        let rules = Rules::new(&t, RulePolicy::default());
        for g in 0..t.len() as Gid {
            for x in 0..t.len() as Gid {
                let fb = rules.feedback(&t, g, x);
                let back = Fb::from_parts(fb.mask(), fb.frame_reveal(), fb.level_reveal()).unwrap();
                assert_eq!(back, fb);
            }
        }
    }
}
