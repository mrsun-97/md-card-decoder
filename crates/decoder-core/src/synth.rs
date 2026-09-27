//! Random small card pools for property tests (tiny value domains force many collisions, so every
//! rule special case — pendulum frames, rule levels, '?' stats, Link DEF — shows up often).

use crate::data::{CardTable, RawGroup, STAT_NONE, STAT_UNKNOWN};
use crate::rng::Rng;

/// A random table with at most `n` groups (duplicates of the judged fields are merged).
pub fn random_table(rng: &mut Rng, n: usize) -> CardTable {
    const FRAMES: [u8; 9] = [1, 2, 2, 2, 4, 16, 32, 130, 132];
    let mut groups: Vec<RawGroup> = Vec::new();
    let mut tries = 0;
    while groups.len() < n && tries < n * 20 {
        tries += 1;
        let frame = FRAMES[rng.below(FRAMES.len() as u64) as usize];
        let level_n = 1 + rng.below(3) as u8;
        // Occasionally a "rule level ≠ printed level" card (printed 0).
        let level_nm = (1u16 << level_n) | if rng.below(6) == 0 { 1 } else { 0 };
        let stat = |r: &mut Rng| [0i16, 1000, 1000, STAT_UNKNOWN][r.below(4) as usize];
        let atk = stat(rng);
        let def = if frame == 32 { STAT_NONE } else { stat(rng) };
        let g = RawGroup {
            frame,
            attribute: [1u8, 16, 32][rng.below(3) as usize],
            race: [1u32, 2, 8192][rng.below(3) as usize],
            level_n,
            level_nm,
            atk,
            def,
            wm: 1 + rng.below(3) as u8,
            wa: 3,
            passcodes: vec![groups.len() as u32 + 1],
            rep_passcode: groups.len() as u32 + 1,
        };
        let same = |o: &RawGroup| {
            (o.frame, o.attribute, o.race, o.level_n, o.level_nm, o.atk, o.def)
                == (g.frame, g.attribute, g.race, g.level_n, g.level_nm, g.atk, g.def)
        };
        if !groups.iter().any(same) {
            groups.push(g);
        }
    }
    CardTable::from_raw(&groups).expect("synthetic table is valid")
}
