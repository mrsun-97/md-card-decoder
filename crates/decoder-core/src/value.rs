//! Lexicographic value vectors: (E[puzzles solved], E[first-time matched fields], −E[actions]).
//! Expectation is linear and the lexicographic order is preserved by addition and positive
//! scaling, so expectimax over these vectors is well defined.

use std::cmp::Ordering;
use std::ops::{Add, AddAssign, Mul};

/// Tolerance for treating two values as equal (values are at most ~10 and ~60 in magnitude).
pub const EPS: f64 = 1e-9;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Val(pub [f64; 3]);

impl Val {
    pub const ZERO: Val = Val([0.0; 3]);

    #[inline]
    pub const fn new(solved: f64, matches: f64, neg_actions: f64) -> Val {
        Val([solved, matches, neg_actions])
    }

    /// Lexicographic comparison with tolerance [`EPS`] per component.
    #[inline]
    pub fn cmp_eps(&self, other: &Val) -> Ordering {
        for i in 0..3 {
            let d = self.0[i] - other.0[i];
            if d > EPS {
                return Ordering::Greater;
            }
            if d < -EPS {
                return Ordering::Less;
            }
        }
        Ordering::Equal
    }

    /// Exact lexicographic total order (for sorting).
    pub fn total_cmp(&self, other: &Val) -> Ordering {
        self.0[0]
            .total_cmp(&other.0[0])
            .then(self.0[1].total_cmp(&other.0[1]))
            .then(self.0[2].total_cmp(&other.0[2]))
    }

    #[inline]
    pub fn gt(&self, other: &Val) -> bool {
        self.cmp_eps(other) == Ordering::Greater
    }

    #[inline]
    pub fn solved(&self) -> f64 {
        self.0[0]
    }
}

impl Add for Val {
    type Output = Val;
    #[inline]
    fn add(self, o: Val) -> Val {
        Val([self.0[0] + o.0[0], self.0[1] + o.0[1], self.0[2] + o.0[2]])
    }
}

impl AddAssign for Val {
    #[inline]
    fn add_assign(&mut self, o: Val) {
        for i in 0..3 {
            self.0[i] += o.0[i];
        }
    }
}

impl Mul<f64> for Val {
    type Output = Val;
    #[inline]
    fn mul(self, p: f64) -> Val {
        Val([self.0[0] * p, self.0[1] * p, self.0[2] * p])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicographic_order() {
        let a = Val::new(1.0, 0.0, -5.0);
        let b = Val::new(0.999, 6.0, 0.0);
        assert!(a.gt(&b));
        assert_eq!(Val::new(1.0, 2.0, -3.0).cmp_eps(&Val::new(1.0 + 1e-12, 2.0, -3.0)), Ordering::Equal);
        assert!(Val::new(1.0, 2.0, -2.0).gt(&Val::new(1.0, 2.0, -3.0)));
    }
}
