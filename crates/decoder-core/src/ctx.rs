//! The immutable context every solver works in: card table, compiled rules and pool weights.

use crate::data::{CardTable, Gid, Pool};
use crate::rules::{RulePolicy, Rules};

pub struct Ctx {
    pub table: CardTable,
    pub rules: Rules,
    pub pool: Pool,
    /// Prior weight of each group as a target (index = group id; 0 = never a target).
    pub weights: Vec<u32>,
    /// Groups with positive weight: the possible targets and the guess universe.
    pub pool_groups: Vec<Gid>,
    /// Full-match lists: pool groups a guess solves (≤ 3 each), CSR layout.
    fm_off: Vec<u32>,
    fm: Vec<Gid>,
    /// Per-group random keys for 128-bit Zobrist hashing of candidate sets.
    pub zob: Vec<u128>,
    /// Per-group `w·log2(w)` (entropy of a set = log2 m − Σ w log2 w / m).
    pub wlw: Vec<f64>,
}

impl Ctx {
    pub fn new(table: CardTable, policy: RulePolicy, pool: Pool) -> Ctx {
        let rules = Rules::new(&table, policy);
        let weights: Vec<u32> = (0..table.len() as Gid).map(|g| table.weight(pool, g)).collect();
        let pool_groups = table.pool_groups(pool);
        let (fm_off, fm) = full_match_lists(&table, &rules, &pool_groups);
        let mut rng = crate::rng::Rng::new(0x5a0b_1a57);
        let zob = (0..table.len()).map(|_| (rng.next_u64() as u128) << 64 | rng.next_u64() as u128).collect();
        let wlw =
            weights.iter().map(|&w| if w > 0 { w as f64 * libm::log2(w as f64) } else { 0.0 }).collect();
        Ctx { table, rules, pool, weights, pool_groups, fm_off, fm, zob, wlw }
    }

    /// 128-bit Zobrist hash of a candidate set.
    #[inline]
    pub fn set_hash(&self, cands: &[Gid]) -> u128 {
        cands.iter().fold(0, |acc, &g| acc ^ self.zob[g as usize])
    }

    /// Posterior entropy (bits) of a candidate set.
    pub fn entropy(&self, cands: &[Gid]) -> f64 {
        let (m, s) = cands
            .iter()
            .fold((0u64, 0.0), |(m, s), &g| (m + self.weights[g as usize] as u64, s + self.wlw[g as usize]));
        set_entropy(m, s)
    }

    /// Pool groups that guess `g` would solve (itself included when it is in the pool).
    #[inline]
    pub fn full_matches(&self, g: Gid) -> &[Gid] {
        &self.fm[self.fm_off[g as usize] as usize..self.fm_off[g as usize + 1] as usize]
    }

    #[inline]
    pub fn mass(&self, cands: &[Gid]) -> u64 {
        cands.iter().map(|&g| self.weights[g as usize] as u64).sum()
    }
}

/// A full match needs equal attribute, race, ATK and DEF, so bucket by those and only test frame and
/// level inside a bucket.
fn full_match_lists(table: &CardTable, rules: &Rules, pool: &[Gid]) -> (Vec<u32>, Vec<Gid>) {
    use std::collections::HashMap;
    let key = |g: Gid| {
        let g = g as usize;
        (table.attribute[g], table.race[g], table.atk[g], table.def[g])
    };
    let mut buckets: HashMap<(u8, u8, u8, u8), Vec<Gid>> = HashMap::new();
    for &g in pool {
        buckets.entry(key(g)).or_default().push(g);
    }
    let mut off = Vec::with_capacity(table.len() + 1);
    let mut fm = Vec::new();
    off.push(0);
    for g in 0..table.len() as Gid {
        if let Some(bucket) = buckets.get(&key(g)) {
            let row = rules.row(table, g);
            fm.extend(bucket.iter().copied().filter(|&t| row.feedback(table, t).solved()));
        }
        off.push(fm.len() as u32);
    }
    (off, fm)
}

/// Entropy (bits) of a weighted set from its mass `m` and `Σ w·log2 w`.
#[inline]
pub fn set_entropy(m: u64, wlw: f64) -> f64 {
    if m == 0 { 0.0 } else { (libm::log2(m as f64) - wlw / m as f64).max(0.0) }
}
