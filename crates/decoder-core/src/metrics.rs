//! O(|C| + U) guess metrics from per-field histograms of the candidate set:
//! solve mass, match mass per field (→ expected new matches) and an upper bound on the feedback
//! entropy, H(outcome) ≤ Σ_f H(response_f) (subadditivity), used to find the exact top-K most
//! informative guesses with few full partitions.

use crate::ctx::Ctx;
use crate::data::{Field, Gid};
use crate::kernel::{Cell, Partitioner};

/// Epoch-marked membership set over group ids (O(1) clear).
#[derive(Clone, Debug, Default)]
pub struct Marks {
    mark: Vec<u32>,
    epoch: u32,
}

impl Marks {
    pub fn set(&mut self, n: usize, members: &[Gid]) {
        if self.mark.len() < n {
            self.mark = vec![0; n];
            self.epoch = 0;
        }
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.mark.fill(0);
            self.epoch = 1;
        }
        for &g in members {
            self.mark[g as usize] = self.epoch;
        }
    }

    #[inline]
    pub fn has(&self, g: Gid) -> bool {
        self.mark[g as usize] == self.epoch
    }
}

/// Weighted histograms of the candidate set, one per field, keyed by value id (level: match id).
#[derive(Clone, Debug, Default)]
pub struct FieldHist {
    pub total: u64,
    pub frame: [u64; 16],
    pub level: [u64; 16],
    pub attribute: Vec<u64>,
    pub race: Vec<u64>,
    pub atk: Vec<u64>,
    pub def: Vec<u64>,
}

impl FieldHist {
    pub fn new(ctx: &Ctx, cands: &[Gid]) -> FieldHist {
        let d = &ctx.table.dicts;
        let t = &ctx.table;
        let mut h = FieldHist {
            attribute: vec![0; d.attribute.len()],
            race: vec![0; d.race.len()],
            atk: vec![0; d.atk.len()],
            def: vec![0; d.def.len()],
            ..Default::default()
        };
        for &g in cands {
            let (gu, w) = (g as usize, ctx.weights[g as usize] as u64);
            h.total += w;
            h.frame[t.frame[gu] as usize] += w;
            h.level[t.level[gu] as usize] += w;
            h.attribute[t.attribute[gu] as usize] += w;
            h.race[t.race[gu] as usize] += w;
            h.atk[t.atk[gu] as usize] += w;
            h.def[t.def[gu] as usize] += w;
        }
        h
    }

    /// Mass of the candidates showing `shown` on `field` (hint outcome masses).
    pub fn shown_masses(&self, ctx: &Ctx, field: Field) -> Vec<(u8, u64)> {
        let v: Vec<(u8, u64)> = match field {
            Field::Frame => self.frame.iter().enumerate().map(|(i, &m)| (i as u8, m)).collect(),
            Field::Level => {
                let mut by_n = [0u64; 16];
                for (id, &m) in self.level.iter().enumerate() {
                    if m > 0 {
                        by_n[ctx.table.level_n_of_id(id as u8) as usize] += m;
                    }
                }
                by_n.iter().enumerate().map(|(n, &m)| (n as u8, m)).collect()
            }
            Field::Attribute => self.attribute.iter().enumerate().map(|(i, &m)| (i as u8, m)).collect(),
            Field::Race => self.race.iter().enumerate().map(|(i, &m)| (i as u8, m)).collect(),
            Field::Atk => self.atk.iter().enumerate().map(|(i, &m)| (i as u8, m)).collect(),
            Field::Def => self.def.iter().enumerate().map(|(i, &m)| (i as u8, m)).collect(),
        };
        v.into_iter().filter(|&(_, m)| m > 0).collect()
    }
}

/// Per-guess summary computed without partitioning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quick {
    pub guess: Gid,
    /// Weight of candidates this guess fully matches (probability of solving now × total).
    pub solve_mass: u64,
    /// Weight of candidates matching each field (field-bit order).
    pub match_mass: [u64; 6],
    /// Upper bound on the entropy of the feedback, in bits.
    pub entropy_ub: f64,
}

impl Quick {
    /// Expected number of first-time matched fields (× total mass).
    pub fn new_match_mass(&self, matched: u8) -> u64 {
        (0..6).filter(|&f| matched & (1 << f) == 0).map(|f| self.match_mass[f]).sum()
    }
}

#[inline]
fn plogp(m: u64, total: f64) -> f64 {
    if m == 0 {
        0.0
    } else {
        let p = m as f64 / total;
        -p * libm::log2(p)
    }
}

/// Entropy (bits) of a partition given cell masses.
pub fn entropy(cells: &[Cell], total: u64) -> f64 {
    let t = total as f64;
    cells.iter().map(|c| plogp(c.mass, t)).sum()
}

/// Quick metrics of one guess. `in_c` must mark the candidate set of `h`.
pub fn quick(ctx: &Ctx, h: &FieldHist, in_c: &Marks, g: Gid) -> Quick {
    let t = &ctx.table;
    let gu = g as usize;
    let total = h.total as f64;
    let w_self = if in_c.has(g) { ctx.weights[gu] as u64 } else { 0 };
    let mut q = Quick { guess: g, ..Default::default() };
    // Frame: cells per matched target frame id, plus one no-match cell.
    let (mut fm, mut fh) = (0u64, 0.0);
    for (tf, &m) in h.frame.iter().enumerate() {
        if m > 0 && ctx.rules.frame_code(t.frame[gu], tf as u8) != 0 {
            fm += m;
            fh += plogp(m, total);
        }
    }
    q.match_mass[0] = fm;
    let mut hub = fh + plogp(h.total - fm, total);
    // Level: matched cells split by revealed rule level.
    let mut by_n = [0u64; 16];
    for (tl, &m) in h.level.iter().enumerate() {
        if m > 0 {
            let code = ctx.rules.level_code(t.level[gu], tl as u8);
            if code != 0 {
                by_n[code as usize] += m;
            }
        }
    }
    let lm: u64 = by_n.iter().sum();
    q.match_mass[3] = lm;
    hub += by_n.iter().map(|&m| plogp(m, total)).sum::<f64>() + plogp(h.total - lm, total);
    // Equality fields (binary responses).
    let eq = |hist: &[u64], id: u8, strict: bool| if strict { w_self } else { hist[id as usize] };
    q.match_mass[1] = eq(&h.attribute, t.attribute[gu], false);
    q.match_mass[2] = eq(&h.race, t.race[gu], false);
    q.match_mass[4] = eq(&h.atk, t.atk[gu], ctx.rules.atk_is_strict(t.atk[gu]));
    q.match_mass[5] = eq(&h.def, t.def[gu], ctx.rules.def_is_strict(t.def[gu]));
    for f in [1, 2, 4, 5] {
        hub += plogp(q.match_mass[f], total) + plogp(h.total - q.match_mass[f], total);
    }
    q.entropy_ub = hub;
    q.solve_mass =
        ctx.full_matches(g).iter().filter(|&&x| in_c.has(x)).map(|&x| ctx.weights[x as usize] as u64).sum();
    q
}

/// Quick metrics for every guess in `universe`.
pub fn quick_all(ctx: &Ctx, cands: &[Gid], universe: &[Gid], in_c: &mut Marks) -> (FieldHist, Vec<Quick>) {
    let h = FieldHist::new(ctx, cands);
    in_c.set(ctx.table.len(), cands);
    let qs = universe.iter().map(|&g| quick(ctx, &h, in_c, g)).collect();
    (h, qs)
}

/// Exact top-`k` guesses by feedback entropy, using the upper bound to skip most partitions.
/// Returns (guess, exact entropy) sorted by entropy descending, ties by guess id.
pub fn top_k_entropy(
    ctx: &Ctx,
    cands: &[Gid],
    quick: &[Quick],
    k: usize,
    part: &mut Partitioner,
) -> Vec<(Gid, f64)> {
    let total = ctx.mass(cands);
    let mut order: Vec<&Quick> = quick.iter().collect();
    order.sort_by(|a, b| b.entropy_ub.total_cmp(&a.entropy_ub).then(a.guess.cmp(&b.guess)));
    let mut best: Vec<(Gid, f64)> = Vec::with_capacity(k + 1);
    let mut cells = Vec::new();
    for q in order {
        if best.len() == k && q.entropy_ub < best[k - 1].1 - 1e-12 {
            break;
        }
        part.cells(ctx, q.guess, cands, &mut cells);
        let e = entropy(&cells, total);
        let pos = best.partition_point(|&(g, x)| x > e || (x == e && g < q.guess));
        if pos < k {
            best.insert(pos, (q.guess, e));
            best.truncate(k);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::tests::tiny_groups;
    use crate::data::{CardTable, Pool};
    use crate::rules::RulePolicy;

    #[test]
    fn quick_matches_partitions() {
        let ctx = Ctx::new(CardTable::from_raw(&tiny_groups()).unwrap(), RulePolicy::default(), Pool::All);
        let cands: Vec<Gid> = (0..ctx.table.len() as Gid).collect();
        let mut marks = Marks::default();
        let (h, qs) = quick_all(&ctx, &cands, &cands, &mut marks);
        let mut part = Partitioner::default();
        let mut cells = Vec::new();
        for q in &qs {
            part.cells(&ctx, q.guess, &cands, &mut cells);
            let exact = entropy(&cells, h.total);
            assert!(q.entropy_ub >= exact - 1e-12, "bound violated for {}", q.guess);
            let solve: u64 = cells.iter().filter(|c| c.fb.solved()).map(|c| c.mass).sum();
            assert_eq!(q.solve_mass, solve);
            for f in 0..6 {
                let m: u64 = cells.iter().filter(|c| c.fb.mask() & (1 << f) != 0).map(|c| c.mass).sum();
                assert_eq!(q.match_mass[f], m, "field {f} guess {}", q.guess);
            }
        }
        let top = top_k_entropy(&ctx, &cands, &qs, 3, &mut part);
        let mut all: Vec<f64> = qs
            .iter()
            .map(|q| {
                part.cells(&ctx, q.guess, &cands, &mut cells);
                entropy(&cells, h.total)
            })
            .collect();
        all.sort_by(|a, b| b.total_cmp(a));
        for (i, (_, e)) in top.iter().enumerate() {
            assert!((e - all[i]).abs() < 1e-12);
        }
    }
}
