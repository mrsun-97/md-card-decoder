//! Hot kernels: partitioning a candidate set by the feedback of one guess.
//!
//! A dense histogram over the 4096 feedback codes plus a "touched" list replaces the string-keyed
//! maps of the legacy solver; cells come out in first-occurrence order and children are written
//! with a counting sort into a caller-owned arena (ascending candidate order is preserved).

use crate::ctx::Ctx;
use crate::data::Gid;
use crate::rules::{FB_CODES, Fb};

/// One outcome cell of a partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub fb: Fb,
    /// Total weight of the candidates in the cell.
    pub mass: u64,
    /// Number of candidates in the cell.
    pub count: u32,
    /// Start of the cell's candidates in the arena (only valid after [`Partitioner::split`]).
    pub start: u32,
}

/// Outcome cell with the entropy ingredients of its candidate set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellE {
    pub fb: Fb,
    pub mass: u64,
    pub count: u32,
    /// Σ w·log2 w over the cell's candidates.
    pub wlw: f64,
}

/// Reusable scratch space for partitions. Not thread-safe; one per worker.
pub struct Partitioner {
    wlw: Vec<f64>,
    mass: Vec<u32>,
    count: Vec<u32>,
    touched: Vec<u16>,
    slot: Vec<u32>,
    codes: Vec<u16>,
    cursor: Vec<u32>,
}

const EMPTY: u32 = u32::MAX;

impl Default for Partitioner {
    fn default() -> Self {
        Partitioner {
            wlw: vec![0.0; FB_CODES],
            mass: vec![0; FB_CODES],
            count: vec![0; FB_CODES],
            touched: Vec::with_capacity(64),
            slot: vec![EMPTY; FB_CODES],
            codes: Vec::new(),
            cursor: Vec::new(),
        }
    }
}

impl Partitioner {
    /// Outcome cells of guess `g` over `cands` (masses and counts only), in first-occurrence order.
    pub fn cells(&mut self, ctx: &Ctx, g: Gid, cands: &[Gid], out: &mut Vec<Cell>) {
        out.clear();
        self.touched.clear();
        let (rules, weights) = (&ctx.rules, &ctx.weights);
        let row = rules.row(&ctx.table, g);
        for &t in cands {
            let code = row.feedback_packed(rules, t).0 as usize & (FB_CODES - 1);
            if self.count[code] == 0 {
                self.touched.push(code as u16);
            }
            self.count[code] += 1;
            self.mass[code] += weights[t as usize];
        }
        for &code in &self.touched {
            let c = code as usize;
            out.push(Cell { fb: Fb(code), mass: self.mass[c] as u64, count: self.count[c], start: 0 });
            self.mass[c] = 0;
            self.count[c] = 0;
        }
    }

    /// Like [`Partitioner::cells`] plus Σ w·log2 w per cell (for child entropies without splitting).
    pub fn cells_ent(&mut self, ctx: &Ctx, g: Gid, cands: &[Gid], out: &mut Vec<CellE>) {
        out.clear();
        self.touched.clear();
        let rules = &ctx.rules;
        let row = rules.row(&ctx.table, g);
        for &t in cands {
            let code = row.feedback_packed(rules, t).0 as usize & (FB_CODES - 1);
            if self.count[code] == 0 {
                self.touched.push(code as u16);
            }
            self.count[code] += 1;
            self.mass[code] += ctx.weights[t as usize];
            self.wlw[code] += ctx.wlw[t as usize];
        }
        for &code in &self.touched {
            let c = code as usize;
            out.push(CellE {
                fb: Fb(code),
                mass: self.mass[c] as u64,
                count: self.count[c],
                wlw: self.wlw[c],
            });
            self.mass[c] = 0;
            self.count[c] = 0;
            self.wlw[c] = 0.0;
        }
    }

    /// Like [`Partitioner::cells`] but also writes each cell's candidates, ascending, into `arena`
    /// (appended); `Cell::start` is relative to the arena length before the call.
    pub fn split(&mut self, ctx: &Ctx, g: Gid, cands: &[Gid], out: &mut Vec<Cell>, arena: &mut Vec<Gid>) {
        out.clear();
        self.codes.clear();
        let (rules, weights) = (&ctx.rules, &ctx.weights);
        let row = rules.row(&ctx.table, g);
        for &t in cands {
            let fb = row.feedback_packed(rules, t);
            self.codes.push(fb.0);
            let s = &mut self.slot[fb.0 as usize];
            if *s == EMPTY {
                *s = out.len() as u32;
                out.push(Cell { fb, mass: 0, count: 0, start: 0 });
            }
            let c = &mut out[*s as usize];
            c.mass += weights[t as usize] as u64;
            c.count += 1;
        }
        let base = arena.len() as u32;
        let mut next = base;
        for c in out.iter_mut() {
            c.start = next;
            next += c.count;
        }
        arena.resize(next as usize, 0);
        // Reuse `slot` as the per-cell write cursor.
        for (i, c) in out.iter().enumerate() {
            self.slot[c.fb.0 as usize] = i as u32;
        }
        self.cursor.clear();
        self.cursor.extend(out.iter().map(|c| c.start));
        for (&t, &code) in cands.iter().zip(&self.codes) {
            let i = self.slot[code as usize] as usize;
            arena[self.cursor[i] as usize] = t;
            self.cursor[i] += 1;
        }
        for c in out.iter() {
            self.slot[c.fb.0 as usize] = EMPTY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::tests::tiny_groups;
    use crate::data::{CardTable, Pool};
    use crate::rules::RulePolicy;

    #[test]
    fn split_matches_naive_grouping() {
        let ctx = Ctx::new(CardTable::from_raw(&tiny_groups()).unwrap(), RulePolicy::default(), Pool::All);
        let (t, rules, w) = (&ctx.table, &ctx.rules, &ctx.weights);
        let cands: Vec<Gid> = (0..t.len() as Gid).collect();
        let mut p = Partitioner::default();
        for g in 0..t.len() as Gid {
            let (mut cells, mut arena) = (Vec::new(), Vec::new());
            p.split(&ctx, g, &cands, &mut cells, &mut arena);
            let mut cells2 = Vec::new();
            p.cells(&ctx, g, &cands, &mut cells2);
            assert_eq!(cells.len(), cells2.len());
            let total: u64 = cells.iter().map(|c| c.mass).sum();
            assert_eq!(total, w.iter().map(|&x| x as u64).sum::<u64>());
            for c in &cells {
                let members = &arena[c.start as usize..(c.start + c.count) as usize];
                assert!(members.windows(2).all(|p| p[0] < p[1]));
                for &m in members {
                    assert_eq!(rules.feedback(t, g, m), c.fb);
                }
            }
        }
    }
}
