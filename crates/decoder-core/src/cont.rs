//! Value models fitted offline (PLAN.md §4.5, §4.7):
//!
//! - [`ContTable`] F̂[k][h][c] = expected (puzzles solved, matched fields) of the `k` puzzles still
//!   to come when the next one starts with `h` hints and `c` challenges. Because every puzzle starts
//!   from the same prior, the event value decomposes exactly into this table.
//! - [`CostToGo`] the learned leaf: the distribution of (challenges, hints) still needed to solve the
//!   current puzzle given its posterior entropy, composed analytically with F̂ so that the
//!   hint-versus-challenge trade-off comes from the fitted table rather than a fixed ratio.

use std::cell::RefCell;

use rustc_hash::FxHashMap;

use crate::search::Continuation;
use crate::value::Val;

/// F̂[k][h][c] → (S, M). `k = 0` or `c = 0` is always zero; `h`, `c` beyond the grid are clamped.
#[derive(Clone, Debug, PartialEq)]
pub struct ContTable {
    pub kmax: u8,
    pub hmax: u8,
    pub cmax: u8,
    s: Vec<f32>,
    m: Vec<f32>,
}

impl ContTable {
    pub fn zeros(kmax: u8, hmax: u8, cmax: u8) -> ContTable {
        let n = (kmax as usize + 1) * (hmax as usize + 1) * (cmax as usize + 1);
        ContTable { kmax, hmax, cmax, s: vec![0.0; n], m: vec![0.0; n] }
    }

    /// The legacy heuristic `min(k, c, (c + 0.61h) / 3.9)` (× 6 matches), as a table.
    pub fn legacy(kmax: u8, hmax: u8, cmax: u8) -> ContTable {
        let mut t = ContTable::zeros(kmax, hmax, cmax);
        for k in 1..=kmax {
            for h in 0..=hmax {
                for c in 1..=cmax {
                    let s = (k as f64).min(c as f64).min((c as f64 + 0.61 * h as f64) / 3.9);
                    t.set(k, h, c, s, 6.0 * s);
                }
            }
        }
        t
    }

    #[inline]
    fn idx(&self, k: u8, h: u8, c: u8) -> usize {
        ((k as usize) * (self.hmax as usize + 1) + h as usize) * (self.cmax as usize + 1) + c as usize
    }

    #[inline]
    pub fn get(&self, k: u8, h: u8, c: u8) -> (f64, f64) {
        if k == 0 || c == 0 {
            return (0.0, 0.0);
        }
        let i = self.idx(k.min(self.kmax), h.min(self.hmax), c.min(self.cmax));
        (self.s[i] as f64, self.m[i] as f64)
    }

    pub fn set(&mut self, k: u8, h: u8, c: u8, s: f64, m: f64) {
        let i = self.idx(k, h, c);
        self.s[i] = s as f32;
        self.m[i] = m as f32;
    }

    /// Enforce monotonicity in c and h (more resources never hurt: free disposal) and the bounds
    /// 0 ≤ S ≤ min(k, c), 0 ≤ M ≤ 6·min(k, c).
    pub fn repair(&mut self) {
        for k in 1..=self.kmax {
            for h in 0..=self.hmax {
                for c in 1..=self.cmax {
                    let cap = (k.min(c)) as f64;
                    let (mut s, mut m) = self.get(k, h, c);
                    let (ps, pm) = self.get(k, h, c - 1);
                    s = s.max(ps).clamp(0.0, cap);
                    m = m.max(pm).clamp(0.0, 6.0 * cap);
                    if h > 0 {
                        let (hs, hm) = self.get(k, h - 1, c);
                        s = s.max(hs).min(cap);
                        m = m.max(hm).min(6.0 * cap);
                    }
                    self.set(k, h, c, s, m);
                }
            }
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"MDCT");
        b.extend_from_slice(&[1, self.kmax, self.hmax, self.cmax]);
        for v in self.s.iter().chain(&self.m) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<ContTable, String> {
        if b.len() < 8 || &b[..4] != b"MDCT" || b[4] != 1 {
            return Err("not a continuation table".into());
        }
        let mut t = ContTable::zeros(b[5], b[6], b[7]);
        let n = t.s.len();
        if b.len() != 8 + 8 * n {
            return Err("continuation table has the wrong size".into());
        }
        let f = |i: usize| f32::from_le_bytes(b[8 + 4 * i..12 + 4 * i].try_into().unwrap());
        for i in 0..n {
            t.s[i] = f(i);
            t.m[i] = f(n + i);
        }
        Ok(t)
    }
}

/// F̂ seen from inside a puzzle with `k` puzzles still to come after it.
pub struct TableCont<'a> {
    pub table: &'a ContTable,
    pub k: u8,
}

impl Continuation for TableCont<'_> {
    #[inline]
    fn value(&self, hints: u8, challenges: u8) -> Val {
        let (s, m) = self.table.get(self.k, hints, challenges);
        Val::new(s, m, 0.0)
    }
    fn is_zero(&self) -> bool {
        self.k == 0
    }
}

/// Entropy buckets of the leaf model: `round(2·H)` for H in bits, capped.
pub const LEAF_BINS: usize = 28;
/// Challenges-still-needed support: 1..=J_MAX.
pub const J_MAX: usize = 16;
/// Hints-still-used support: 0..=E_MAX.
pub const E_MAX: usize = 6;

#[inline]
pub fn entropy_bin(h_bits: f64) -> usize {
    ((h_bits * 2.0).round().max(0.0) as usize).min(LEAF_BINS - 1)
}

/// P(J challenges, E hints still needed to solve | entropy bin, hints available?).
#[derive(Clone, Debug, PartialEq)]
pub struct CostToGo {
    /// `[bin][flag][j-1][e]`, rows sum to 1.
    p: Vec<f32>,
}

impl CostToGo {
    #[inline]
    fn idx(bin: usize, flag: usize, j: usize, e: usize) -> usize {
        ((bin * 2 + flag) * J_MAX + (j - 1)) * (E_MAX + 1) + e
    }

    /// Fit from samples `(entropy bits, hints available, J, E)` of solved puzzles. Sparse cells
    /// borrow from the other flag, then from neighbouring bins, until `min_samples` are pooled.
    pub fn fit(samples: &[(f64, bool, u8, u8)], min_samples: usize) -> CostToGo {
        let mut counts = vec![0u32; LEAF_BINS * 2 * J_MAX * (E_MAX + 1)];
        let mut totals = [[0u32; 2]; LEAF_BINS];
        for &(h, flag, j, e) in samples {
            let (j, e) = ((j as usize).clamp(1, J_MAX), (e as usize).min(E_MAX));
            let b = entropy_bin(h);
            counts[Self::idx(b, flag as usize, j, e)] += 1;
            totals[b][flag as usize] += 1;
        }
        let mut p = vec![0f32; counts.len()];
        for b in 0..LEAF_BINS {
            for flag in 0..2 {
                // Sources in order: same cell, other flag, then widening bin neighbourhoods.
                let mut sources: Vec<(usize, usize)> = vec![(b, flag)];
                let mut n: u32 = totals[b][flag];
                if (n as usize) < min_samples {
                    sources.push((b, 1 - flag));
                    n += totals[b][1 - flag];
                }
                let mut radius = 1;
                while (n as usize) < min_samples && radius < LEAF_BINS {
                    for nb in [b.wrapping_sub(radius), b + radius] {
                        if nb < LEAF_BINS {
                            for f in [flag, 1 - flag] {
                                sources.push((nb, f));
                                n += totals[nb][f];
                            }
                        }
                    }
                    radius += 1;
                }
                if n == 0 {
                    p[Self::idx(b, flag, 1, 0)] = 1.0;
                    continue;
                }
                for j in 1..=J_MAX {
                    for e in 0..=E_MAX {
                        let c: u32 = sources.iter().map(|&(sb, sf)| counts[Self::idx(sb, sf, j, e)]).sum();
                        p[Self::idx(b, flag, j, e)] = c as f32 / n as f32;
                    }
                }
            }
        }
        CostToGo { p }
    }

    #[inline]
    pub fn prob(&self, bin: usize, flag: bool, j: usize, e: usize) -> f64 {
        self.p[Self::idx(bin, flag as usize, j, e)] as f64
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = b"MDCL".to_vec();
        b.extend_from_slice(&[1, LEAF_BINS as u8, J_MAX as u8, E_MAX as u8]);
        for v in &self.p {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<CostToGo, String> {
        if b.len() < 8 || &b[..4] != b"MDCL" || b[4] != 1 {
            return Err("not a leaf table".into());
        }
        if (b[5] as usize, b[6] as usize, b[7] as usize) != (LEAF_BINS, J_MAX, E_MAX) {
            return Err("leaf table dimensions differ".into());
        }
        let n = LEAF_BINS * 2 * J_MAX * (E_MAX + 1);
        if b.len() != 8 + 4 * n {
            return Err("leaf table has the wrong size".into());
        }
        Ok(CostToGo {
            p: (0..n).map(|i| f32::from_le_bytes(b[8 + 4 * i..12 + 4 * i].try_into().unwrap())).collect(),
        })
    }
}

/// State summary a leaf evaluator sees.
#[derive(Clone, Copy, Debug)]
pub struct LeafState {
    pub mass: u64,
    /// Posterior entropy of the candidate set, in bits.
    pub entropy: f64,
    pub known: u8,
    pub matched: u8,
    pub hints: u8,
    pub challenges: u8,
}

/// Estimated value of an unsolved node at the search horizon.
pub trait Leaf {
    fn eval(&self, st: &LeafState) -> Val;
}

/// The legacy horizon value: nothing beyond the search depth.
pub struct ZeroLeaf;

impl Leaf for ZeroLeaf {
    fn eval(&self, _: &LeafState) -> Val {
        Val::ZERO
    }
}

/// Learned leaf: E over (J, E) ~ CostToGo of solving with `c − J`, `h − E` left, then F̂.
pub struct LearnedLeaf<'a> {
    pub ctg: &'a CostToGo,
    pub table: &'a ContTable,
    /// Puzzles still to come after the current one.
    pub k: u8,
    cache: RefCell<FxHashMap<(u8, u8, u8), [f64; 4]>>,
}

impl<'a> LearnedLeaf<'a> {
    pub fn new(ctg: &'a CostToGo, table: &'a ContTable, k: u8) -> LearnedLeaf<'a> {
        LearnedLeaf { ctg, table, k, cache: RefCell::default() }
    }

    /// (P(solve), E[S from future puzzles | solve]·P, E[M from future]·P, E[actions]) for a bin.
    fn row(&self, bin: u8, h: u8, c: u8) -> [f64; 4] {
        if let Some(r) = self.cache.borrow().get(&(bin, h, c)) {
            return *r;
        }
        let mut r = [0.0; 4];
        for j in 1..=J_MAX {
            for e in 0..=E_MAX {
                let p = self.ctg.prob(bin as usize, h > 0, j, e);
                if p == 0.0 {
                    continue;
                }
                r[3] += p * ((j.min(c as usize)) as f64 + e.min(h as usize) as f64);
                if j <= c as usize {
                    let (fs, fm) = self.table.get(self.k, h.saturating_sub(e as u8), c - j as u8);
                    r[0] += p;
                    r[1] += p * fs;
                    r[2] += p * fm;
                }
            }
        }
        self.cache.borrow_mut().insert((bin, h, c), r);
        r
    }
}

impl LearnedLeaf<'_> {
    /// Forecast for the current puzzle: (P(solve it), E[challenges still used], E[hints still used]).
    pub fn forecast(&self, entropy: f64, hints: u8, challenges: u8) -> (f64, f64, f64) {
        let bin = entropy_bin(entropy);
        let (mut p_solve, mut ej, mut ee) = (0.0, 0.0, 0.0);
        for j in 1..=J_MAX {
            for e in 0..=E_MAX {
                let p = self.ctg.prob(bin, hints > 0, j, e);
                ej += p * j.min(challenges as usize) as f64;
                ee += p * e.min(hints as usize) as f64;
                if j <= challenges as usize {
                    p_solve += p;
                }
            }
        }
        (p_solve, ej, ee)
    }
}

impl Leaf for LearnedLeaf<'_> {
    fn eval(&self, st: &LeafState) -> Val {
        if st.challenges == 0 || st.mass == 0 {
            return Val::ZERO;
        }
        let r = self.row(entropy_bin(st.entropy) as u8, st.hints, st.challenges);
        let open = (6 - st.matched.count_ones()) as f64;
        Val::new(r[0] + r[1], open * r[0] + r[2], -r[3])
    }
}

/// Pack the fitted models into one file for the browser: "MDTB" | len u32 | cont | len u32 | leaf.
pub fn pack_tables(cont: &ContTable, leaf: Option<&CostToGo>) -> Vec<u8> {
    let mut b = b"MDTB".to_vec();
    let c = cont.to_bytes();
    b.extend_from_slice(&(c.len() as u32).to_le_bytes());
    b.extend_from_slice(&c);
    let l = leaf.map(|l| l.to_bytes()).unwrap_or_default();
    b.extend_from_slice(&(l.len() as u32).to_le_bytes());
    b.extend_from_slice(&l);
    b
}

pub fn unpack_tables(b: &[u8]) -> Result<(ContTable, Option<CostToGo>), String> {
    if b.len() < 12 || &b[..4] != b"MDTB" {
        return Err("not a tables file".into());
    }
    let len = |at: usize| -> Result<usize, String> {
        b.get(at..at + 4)
            .map(|x| u32::from_le_bytes(x.try_into().unwrap()) as usize)
            .ok_or("truncated tables file".into())
    };
    let cl = len(4)?;
    let cont = ContTable::from_bytes(b.get(8..8 + cl).ok_or("truncated tables file")?)?;
    let ll = len(8 + cl)?;
    let leaf_bytes = b.get(12 + cl..12 + cl + ll).ok_or("truncated tables file")?;
    let leaf = if ll == 0 { None } else { Some(CostToGo::from_bytes(leaf_bytes)?) };
    Ok((cont, leaf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_roundtrip_and_repair() {
        let mut t = ContTable::legacy(3, 4, 10);
        t.set(2, 1, 5, 0.1, 0.1); // a dip that repair must lift
        t.repair();
        assert!(t.get(2, 1, 5).0 >= t.get(2, 1, 4).0);
        assert!(t.get(2, 1, 5).0 >= t.get(2, 0, 5).0);
        assert_eq!(ContTable::from_bytes(&t.to_bytes()).unwrap(), t);
        assert_eq!(t.get(0, 3, 3), (0.0, 0.0));
        assert_eq!(t.get(3, 99, 99), t.get(3, 4, 10));
    }

    #[test]
    fn leaf_fit_normalises_and_roundtrips() {
        let samples: Vec<(f64, bool, u8, u8)> =
            (0..500).map(|i| (i as f64 % 9.0, i % 2 == 0, 1 + (i % 4) as u8, (i % 3) as u8)).collect();
        let ctg = CostToGo::fit(&samples, 40);
        for b in 0..LEAF_BINS {
            for flag in [false, true] {
                let s: f64 = (1..=J_MAX)
                    .flat_map(|j| (0..=E_MAX).map(move |e| (j, e)))
                    .map(|(j, e)| ctg.prob(b, flag, j, e))
                    .sum();
                assert!((s - 1.0).abs() < 1e-5, "bin {b} flag {flag}: {s}");
            }
        }
        assert_eq!(CostToGo::from_bytes(&ctg.to_bytes()).unwrap(), ctg);
        let table = ContTable::legacy(3, 11, 36);
        let leaf = LearnedLeaf::new(&ctg, &table, 2);
        let v =
            leaf.eval(&LeafState { mass: 10, entropy: 2.0, known: 1, matched: 0, hints: 3, challenges: 10 });
        assert!(v.0[0] > 0.9 && v.0[0] <= 3.0 + 1e-9);
    }
}
