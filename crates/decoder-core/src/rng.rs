//! Small deterministic PRNGs (no `getrandom`, identical on native and wasm).

/// SplitMix64: seeding and stream derivation.
#[derive(Clone, Debug)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Mix several integers into one seed (e.g. `mix(&[seed, activity, puzzle, stream])`).
pub fn mix(parts: &[u64]) -> u64 {
    let mut s = SplitMix64(0x6d64_6364_6563_6f64);
    let mut h = 0u64;
    for &p in parts {
        s.0 ^= p.wrapping_add(h);
        h = s.next_u64();
    }
    h
}

/// xoshiro256++: the simulation generator.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut sm = SplitMix64(seed);
        Rng { s: [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()] }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform integer in `0..n` (Lemire's method, unbiased).
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        loop {
            let m = (self.next_u64() as u128) * (n as u128);
            let lo = m as u64;
            if lo >= n.wrapping_neg() % n {
                return (m >> 64) as u64;
            }
        }
    }

    /// Index drawn with probability proportional to `weights[i]` (total must be positive).
    pub fn weighted<T: Copy + Into<u64>>(&mut self, weights: &[T]) -> usize {
        let total: u64 = weights.iter().map(|&w| w.into()).sum();
        let mut x = self.below(total);
        for (i, &w) in weights.iter().enumerate() {
            let w = w.into();
            if x < w {
                return i;
            }
            x -= w;
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1000 {
            let x = a.below(7);
            assert_eq!(x, b.below(7));
            assert!(x < 7);
        }
        assert_ne!(mix(&[1, 2]), mix(&[2, 1]));
        let mut r = Rng::new(1);
        let mut counts = [0u32; 3];
        for _ in 0..30000 {
            counts[r.weighted(&[1u32, 0, 2])] += 1;
        }
        assert_eq!(counts[1], 0);
        assert!((counts[2] as f64 / counts[0] as f64 - 2.0).abs() < 0.1);
    }
}
