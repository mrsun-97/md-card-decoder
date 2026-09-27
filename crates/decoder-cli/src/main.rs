//! `mdcd`: native tooling for the Card Decoder engine.
//!
//! ```text
//! mdcd data build <raw-dir> <out-dir>                rebuild the card table from pinned upstream sources
//! mdcd data compare-legacy <cards-data.js> <bin>     compare a table with the legacy data file
//! mdcd data from-legacy <cards-data.js> <out-dir>    convert the legacy data file (bootstrap)
//! mdcd check-rules <cards-data.js> <cards.bin>       compare judging with the legacy formula on all pairs
//! mdcd bench <cards.bin>                             kernel benchmarks
//! mdcd card <cards.bin> <passcode>...                print the judged fields of cards
//! mdcd bench-exact <cards.bin>                       exact final-puzzle solver on sampled endgames
//! mdcd sim|compare|fit-cont|fit-leaf <cards.bin> key=value...   simulation and fitting (see simcmd.rs)
//! ```

mod build;
mod legacy;
mod simcmd;

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use decoder_core::kernel::{Cell, Partitioner};
use decoder_core::{CardTable, Ctx, Field, Gid, Pool, RulePolicy, Rules};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["data", "build", raw, out] => build::run_build(Path::new(raw), Path::new(out)),
        ["data", "compare-legacy", src, bin] => build::compare_legacy(Path::new(src), Path::new(bin)),
        ["data", "from-legacy", src, out] => legacy::convert(Path::new(src), Path::new(out)),
        ["check-rules", src, bin] => check_rules(Path::new(src), Path::new(bin)),
        ["bench", bin] => bench(Path::new(bin)),
        ["card", bin, rest @ ..] => show_cards(Path::new(bin), rest),
        ["bench-exact", bin] => bench_exact(Path::new(bin)),
        ["sim", bin, rest @ ..] => simcmd::sim(Path::new(bin), rest),
        ["compare", bin, rest @ ..] => simcmd::compare(Path::new(bin), rest),
        ["fit-cont", bin, rest @ ..] => simcmd::fit_cont(Path::new(bin), rest),
        ["fit-leaf", bin, rest @ ..] => simcmd::fit_leaf(Path::new(bin), rest),
        ["bench-decide", bin, rest @ ..] => simcmd::bench_decide(Path::new(bin), rest),
        ["pack-tables", dir] => simcmd::pack_tables(Path::new(dir)),
        _ => {
            eprintln!(
                "usage:\n  mdcd data build <raw-dir> <out-dir>\n  mdcd data compare-legacy <cards-data.js> <cards.bin>\n  mdcd data from-legacy <cards-data.js> <out-dir>\n  mdcd check-rules <cards-data.js> <cards.bin>\n  mdcd bench <cards.bin>"
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) type Res = Result<(), Box<dyn std::error::Error>>;

fn load_table(bin: &Path) -> Result<CardTable, Box<dyn std::error::Error>> {
    Ok(CardTable::from_bytes(&std::fs::read(bin)?)?)
}

/// Judge every (guess, target) pair with the table-driven rules and with the legacy formula
/// (`b&b`, `a==a`, `r==r`, `nm&nm`, `atk==atk`, `def==def`) on the raw legacy values.
fn check_rules(src: &Path, bin: &Path) -> Res {
    let groups = legacy::read_groups(src)?;
    let table = load_table(bin)?;
    let rules = Rules::new(&table, RulePolicy::default());
    // Map legacy index → new group via the first passcode.
    let gid_of: Vec<Gid> = groups
        .iter()
        .map(|g| table.group_of_passcode(g.ids[0]).expect("passcode missing from cards.bin"))
        .collect();
    let started = Instant::now();
    let mut pairs = 0u64;
    let mut mismatches = 0u64;
    for (gi, g) in groups.iter().enumerate() {
        let row = rules.row(&table, gid_of[gi]);
        for (ti, t) in groups.iter().enumerate() {
            let legacy_mask = (g.b & t.b != 0) as u8
                | ((g.a == t.a) as u8) << 1
                | ((g.r == t.r) as u8) << 2
                | ((g.nm & t.nm != 0) as u8) << 3
                | ((g.atk == t.atk) as u8) << 4
                | ((g.def == t.def) as u8) << 5;
            let fb = row.feedback(&table, gid_of[ti]);
            let mut ok = fb.mask() == legacy_mask;
            if let Some(f) = fb.frame_reveal() {
                ok &= table.dicts.frame[f as usize] as u32 == t.b;
            }
            if let Some(n) = fb.level_reveal() {
                ok &= n as u32 == t.n;
            }
            if !ok {
                mismatches += 1;
                if mismatches <= 5 {
                    eprintln!(
                        "mismatch: guess {} target {} legacy {legacy_mask:06b} new {:06b}",
                        g.name,
                        t.name,
                        fb.mask()
                    );
                }
            }
            pairs += 1;
        }
    }
    println!("pairs {pairs}, mismatches {mismatches}, {:.1} s", started.elapsed().as_secs_f64());
    if mismatches > 0 { Err("rule mismatch".into()) } else { Ok(()) }
}

fn bench(bin: &Path) -> Res {
    use decoder_core::metrics::{Marks, entropy, quick_all, top_k_entropy};
    let ctx = Ctx::new(load_table(bin)?, RulePolicy::default(), Pool::MasterDuel);
    let table = &ctx.table;
    let md: Vec<Gid> = ctx.pool_groups.clone();
    println!("groups {}, MD groups {}, data hash {:016x}", table.len(), md.len(), table.data_hash);
    let mut part = Partitioner::default();
    let mut cells: Vec<Cell> = Vec::new();
    let mut marks = Marks::default();
    let states: Vec<(&str, Vec<Gid>)> = vec![
        ("frame=effect", filter(table, &md, Field::Frame, raw(table, Field::Frame, 2))),
        ("level=4", filter(table, &md, Field::Level, 4)),
        ("attribute=dark", filter(table, &md, Field::Attribute, raw(table, Field::Attribute, 32))),
        ("atk=1500", filter(table, &md, Field::Atk, raw(table, Field::Atk, 1500))),
    ];
    for (name, cands) in &states {
        let reps = if cands.len() > 1000 { 3 } else { 20 };
        let started = Instant::now();
        let mut total = 0usize;
        let mut best = (0.0f64, 0 as Gid);
        let mass = ctx.mass(cands);
        for _ in 0..reps {
            for &g in &md {
                part.cells(&ctx, g, cands, &mut cells);
                total += cells.len();
                let e = entropy(&cells, mass);
                if e > best.0 {
                    best = (e, g);
                }
            }
        }
        let secs = started.elapsed().as_secs_f64() / reps as f64;
        let pairs = (md.len() * cands.len()) as f64;
        let t0 = Instant::now();
        let (_, qs) = quick_all(&ctx, cands, &md, &mut marks);
        let quick_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();
        let top = top_k_entropy(&ctx, cands, &qs, 64, &mut part);
        let top_ms = t1.elapsed().as_secs_f64() * 1e3;
        assert!((top[0].1 - best.0).abs() < 1e-9, "top-K disagrees with brute force");
        println!(
            "{name:>15}: n={:>5}  all-guess partition {:>7.2} ms ({:.2} ns/pair, {:.1} cells)  quick_all {:.2} ms  top-64 {:.2} ms  best H {:.3} bits",
            cands.len(),
            secs * 1e3,
            secs * 1e9 / pairs,
            total as f64 / (reps * md.len()) as f64,
            quick_ms,
            top_ms,
            best.0
        );
    }
    Ok(())
}

fn raw(table: &CardTable, field: Field, value: i64) -> u8 {
    table.shown_from_raw(field, value).expect("value not in dictionary")
}

fn filter(table: &CardTable, pool: &[Gid], field: Field, shown: u8) -> Vec<Gid> {
    pool.iter().copied().filter(|&g| table.shown(field, g) == shown).collect()
}

/// Print the judged fields of cards given by passcode (names from names.json next to cards.bin).
fn show_cards(bin: &Path, passcodes: &[&str]) -> Res {
    let t = load_table(bin)?;
    let names: serde_json::Value = std::fs::read(bin.with_file_name("names.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    for pc in passcodes {
        let pc: u32 = pc.parse()?;
        let Some(g) = t.group_of_passcode(pc) else {
            println!("{pc}: not found");
            continue;
        };
        let d = &t.dicts;
        let gu = g as usize;
        println!(
            "{pc} group {g} {}: frame {} attr {} race {} level {} (nm {:#b}) atk {} def {} wm {}",
            names["display"][gu].as_str().unwrap_or("?"),
            d.frame[t.frame[gu] as usize],
            d.attribute[t.attribute[gu] as usize],
            d.race[t.race[gu] as usize],
            t.level_n[gu],
            d.level_nm[t.level[gu] as usize],
            d.atk[t.atk[gu] as usize],
            d.def[t.def[gu] as usize],
            t.wm[gu]
        );
    }
    Ok(())
}

/// Sample endgame states (one reveal, then max-entropy challenges until few candidates remain).
fn sample_endgames(ctx: &Ctx, count: usize, max_n: usize, seed: u64) -> Vec<decoder_core::Belief> {
    use decoder_core::metrics::{Marks, quick_all, top_k_entropy};
    use decoder_core::rng::Rng;
    use decoder_core::{Belief, Event};
    let mut rng = Rng::new(seed);
    let mut part = Partitioner::default();
    let mut marks = Marks::default();
    let weights: Vec<u32> = ctx.pool_groups.iter().map(|&g| ctx.weights[g as usize]).collect();
    let mut out = Vec::new();
    while out.len() < count {
        let target = ctx.pool_groups[rng.weighted(&weights)];
        let f = Field::from_index(rng.below(6) as usize);
        let mut b = Belief::prior(ctx);
        b.apply(ctx, &Event::Reveal { field: f, shown: ctx.table.shown(f, target) }).unwrap();
        while b.cands.len() > max_n {
            let (_, qs) = quick_all(ctx, &b.cands, &ctx.pool_groups, &mut marks);
            let g = top_k_entropy(ctx, &b.cands, &qs, 1, &mut part)[0].0;
            let fb = ctx.rules.feedback(&ctx.table, g, target);
            if fb.solved() {
                break;
            }
            b.apply(ctx, &Event::Challenge { guess: g, fb }).unwrap();
        }
        if b.cands.len() >= 2 && b.cands.len() <= max_n && !b.solved {
            out.push(b);
        }
    }
    out
}

fn bench_exact(bin: &Path) -> Res {
    use decoder_core::search::exact::ExactSolver;
    use decoder_core::search::{NoContinuation, reference::node_of};
    let ctx = Ctx::new(load_table(bin)?, RulePolicy::default(), Pool::MasterDuel);
    let states = sample_endgames(&ctx, 40, 20, 42);
    println!("{:>4} {:>6} {:>7} {:>10} {:>9} {:>10}", "n", "h,c", "solved", "ms", "expanded", "value");
    let mut worst = std::collections::BTreeMap::new();
    let mut checked = 0;
    for b in &states {
        for (h, c) in [(0u8, 3u8), (4, 6), (11, 36)] {
            let node = node_of(b, h, c);
            let t0 = Instant::now();
            let mut ex =
                ExactSolver::new(&ctx, &NoContinuation, &b.cands, &ctx.pool_groups, 2_000_000).unwrap();
            let res = ex.value(&node);
            let ms = t0.elapsed().as_secs_f64() * 1e3;
            if b.cands.len() <= 11
                && let Ok(v) = res
            {
                let mut plain =
                    ExactSolver::new(&ctx, &NoContinuation, &b.cands, &ctx.pool_groups, 5_000_000).unwrap();
                plain.no_bounds = true;
                let w = plain.value(&node).expect("unpruned budget");
                assert_eq!(
                    v.cmp_eps(&w),
                    std::cmp::Ordering::Equal,
                    "B&B differs on real data: {v:?} vs {w:?}"
                );
                checked += 1;
            }
            let bucket = (b.cands.len().div_ceil(4) * 4, h, c);
            let e = worst.entry(bucket).or_insert((0.0f64, 0u64, 0usize));
            if ms > e.0 {
                *e = (ms, ex.expanded, b.cands.len());
            }
            if let Ok(v) = res {
                if b.cands.len() >= 12 {
                    println!(
                        "{:>4} {:>6} {:>7.4} {:>10.2} {:>9} {:>10.4}",
                        b.cands.len(),
                        format!("{h},{c}"),
                        v.0[0],
                        ms,
                        ex.expanded,
                        v.0[2]
                    );
                }
            } else {
                println!("{:>4} {:>6}  budget exhausted after {ms:.0} ms", b.cands.len(), format!("{h},{c}"));
            }
        }
    }
    println!("branch-and-bound equals the unpruned solver on {checked} real endgames (n ≤ 11)");
    println!("worst time per (n bucket, h, c):");
    for ((nb, h, c), (ms, exp, n)) in worst {
        println!("  n≤{nb:<3} h{h:<2} c{c:<2}: {ms:>9.2} ms  ({exp} states, n={n})");
    }
    Ok(())
}
