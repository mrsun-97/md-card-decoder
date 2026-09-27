//! Simulation, paired policy comparison and offline fitting of the value models.
//!
//! ```text
//! mdcd sim <cards.bin> policy=FV preset=9/11/36 n=1000 seed=1 tables=tables
//! mdcd compare <cards.bin> policies=L,FV preset=9/11/28 n=2000 seed=1 tables=tables
//! mdcd fit-cont <cards.bin> out=tables/cont.bin kmax=8 hmax=11 cmax=36 samples=64 seed=777
//! mdcd fit-leaf <cards.bin> out=tables/leaf.bin presets=9/11/36,9/11/28,9/4/30,9/0/36 n=4000 seed=4242
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use decoder_core::cont::{ContTable, CostToGo};
use decoder_core::policy::{Models, Planner, PolicyCfg};
use decoder_core::rng::mix;
use decoder_core::sim::{ActivityOutcome, EventCfg, play_activity, play_puzzle};
use decoder_core::{CardTable, Ctx, Pool, RulePolicy};
use rayon::prelude::*;

use crate::Res;

type Opts = HashMap<String, String>;

pub fn parse_opts(args: &[&str]) -> Result<Opts, String> {
    args.iter()
        .map(|a| {
            a.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("expected key=value, got {a}"))
        })
        .collect()
}

fn opt<T: std::str::FromStr>(o: &Opts, key: &str, default: T) -> Result<T, String> {
    match o.get(key) {
        Some(v) => v.parse().map_err(|_| format!("bad value for {key}: {v}")),
        None => Ok(default),
    }
}

fn preset(s: &str) -> Result<EventCfg, String> {
    let parts: Vec<u8> =
        s.split('/').map(|x| x.parse().map_err(|_| format!("bad preset {s}"))).collect::<Result<_, _>>()?;
    match parts.as_slice() {
        [p, h, c] => Ok(EventCfg { puzzles: *p, hints: *h, challenges: *c }),
        _ => Err(format!("preset must be puzzles/hints/challenges, got {s}")),
    }
}

fn policy(name: &str) -> Result<PolicyCfg, String> {
    Ok(match name {
        "L" => PolicyCfg::legacy(),
        "F" => PolicyCfg::fitted_zero_leaf(),
        "FV" => PolicyCfg::fv2(),
        "FV-noexact" => PolicyCfg { exact: false, ..PolicyCfg::fv2() },
        "FV3" => PolicyCfg { depth: 3, ..PolicyCfg::fv2() },
        "FVK16" => PolicyCfg { extra_entropy: 16, ..PolicyCfg::fv2() },
        "FVK32" => PolicyCfg { extra_entropy: 32, ..PolicyCfg::fv2() },
        other => return Err(format!("unknown policy {other} (L, F, FV, FV-noexact, FV3, FVK16, FVK32)")),
    })
}

fn load_ctx(bin: &Path) -> Result<Ctx, Box<dyn std::error::Error>> {
    Ok(Ctx::new(CardTable::from_bytes(&std::fs::read(bin)?)?, RulePolicy::default(), Pool::MasterDuel))
}

/// Fitted tables from `dir` (cont.bin, leaf.bin); missing files fall back to the legacy formula.
pub fn load_models(dir: &Path) -> Result<Models, Box<dyn std::error::Error>> {
    let mut m = Models::legacy_only();
    if let Ok(b) = std::fs::read(dir.join("cont.bin")) {
        m.cont = ContTable::from_bytes(&b)?;
    }
    if let Ok(b) = std::fs::read(dir.join("leaf.bin")) {
        m.leaf = Some(CostToGo::from_bytes(&b)?);
    }
    Ok(m)
}

fn run_many(
    ctx: &Ctx,
    models: &Models,
    cfg: &PolicyCfg,
    event: EventCfg,
    seed: u64,
    n: u64,
) -> Vec<ActivityOutcome> {
    (0..n)
        .into_par_iter()
        .map_init(Planner::default, |planner, i| {
            play_activity(ctx, models, planner, cfg, event, seed, i, None)
        })
        .collect()
}

struct Summary {
    mean: f64,
    ci: f64,
    p_all: f64,
    p_all_ci: (f64, f64),
    hints: f64,
    challenges: f64,
}

fn summarize(r: &[ActivityOutcome], puzzles: u8) -> Summary {
    let n = r.len() as f64;
    let mean = r.iter().map(|x| x.solved as f64).sum::<f64>() / n;
    let var = r.iter().map(|x| (x.solved as f64 - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let all = r.iter().filter(|x| x.solved == puzzles).count() as f64 / n;
    // Wilson score interval for P(all puzzles solved).
    let z = 1.96f64;
    let centre = (all + z * z / (2.0 * n)) / (1.0 + z * z / n);
    let half = z * (all * (1.0 - all) / n + z * z / (4.0 * n * n)).sqrt() / (1.0 + z * z / n);
    Summary {
        mean,
        ci: 1.96 * (var / n).sqrt(),
        p_all: all,
        p_all_ci: (centre - half, centre + half),
        hints: r.iter().map(|x| x.hints_used as f64).sum::<f64>() / n,
        challenges: r.iter().map(|x| x.challenges_used as f64).sum::<f64>() / n,
    }
}

pub fn sim(bin: &Path, args: &[&str]) -> Res {
    let o = parse_opts(args)?;
    let ctx = load_ctx(bin)?;
    let models = load_models(&PathBuf::from(opt(&o, "tables", "tables".to_string())?))?;
    let cfg = policy(&opt(&o, "policy", "FV".to_string())?)?;
    let event = preset(&opt(&o, "preset", "9/11/36".to_string())?)?;
    let (n, seed) = (opt(&o, "n", 1000u64)?, opt(&o, "seed", 1u64)?);
    let t0 = Instant::now();
    let r = run_many(&ctx, &models, &cfg, event, seed, n);
    let s = summarize(&r, event.puzzles);
    println!(
        "{} {}/{}/{} n={n}: E[solved] {:.4} ± {:.4}, P(all) {:.2}% [{:.2}, {:.2}], hints {:.2}, challenges {:.2} ({:.1} s)",
        opt(&o, "policy", "FV".to_string())?,
        event.puzzles,
        event.hints,
        event.challenges,
        s.mean,
        s.ci,
        100.0 * s.p_all,
        100.0 * s.p_all_ci.0,
        100.0 * s.p_all_ci.1,
        s.hints,
        s.challenges,
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Paired comparison on common random numbers; Δ is against the first policy.
pub fn compare(bin: &Path, args: &[&str]) -> Res {
    let o = parse_opts(args)?;
    let ctx = load_ctx(bin)?;
    let models = load_models(&PathBuf::from(opt(&o, "tables", "tables".to_string())?))?;
    let names: Vec<String> = opt(&o, "policies", "L,FV".to_string())?.split(',').map(String::from).collect();
    let event = preset(&opt(&o, "preset", "9/11/36".to_string())?)?;
    let (n, seed) = (opt(&o, "n", 1000u64)?, opt(&o, "seed", 1u64)?);
    let mut base: Option<Vec<ActivityOutcome>> = None;
    for name in &names {
        let cfg = policy(name)?;
        let t0 = Instant::now();
        let r = run_many(&ctx, &models, &cfg, event, seed, n);
        let s = summarize(&r, event.puzzles);
        let mut line = format!(
            "{name:>10}: E[S] {:.4} ± {:.4}  P(all) {:>6.2}%  hints {:.2}  chal {:.2}  ({:.1} s)",
            s.mean,
            s.ci,
            100.0 * s.p_all,
            s.hints,
            s.challenges,
            t0.elapsed().as_secs_f64()
        );
        if let Some(b) = &base {
            let d: Vec<f64> = r.iter().zip(b).map(|(x, y)| x.solved as f64 - y.solved as f64).collect();
            let da: Vec<f64> = r
                .iter()
                .zip(b)
                .map(|(x, y)| {
                    ((x.solved == event.puzzles) as u8 as f64) - ((y.solved == event.puzzles) as u8 as f64)
                })
                .collect();
            let (m, ci) = mean_ci(&d);
            let (ma, cia) = mean_ci(&da);
            line += &format!("  ΔS {m:+.4} ± {ci:.4}  ΔP(all) {:+.2} ± {:.2} pts", 100.0 * ma, 100.0 * cia);
        } else {
            base = Some(r);
        }
        println!("{line}");
    }
    Ok(())
}

fn mean_ci(d: &[f64]) -> (f64, f64) {
    let n = d.len() as f64;
    let m = d.iter().sum::<f64>() / n;
    let var = d.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (m, 1.96 * (var / n).sqrt())
}

/// Fit F̂ by backward induction: F̂[k][h][c] = E[1{solved}·(1 + F̂[k−1][h'][c'])] over fresh puzzles
/// played by `policy` (default: depth-2 zero leaf, consulting F̂[k−1]), with common random numbers
/// across cells; then monotone repair.
pub fn fit_cont(bin: &Path, args: &[&str]) -> Res {
    let o = parse_opts(args)?;
    let ctx = load_ctx(bin)?;
    let out = PathBuf::from(opt(&o, "out", "tables/cont.bin".to_string())?);
    let (kmax, hmax, cmax) = (opt(&o, "kmax", 8u8)?, opt(&o, "hmax", 11u8)?, opt(&o, "cmax", 36u8)?);
    let samples = opt(&o, "samples", 64u64)?;
    let seed = opt(&o, "seed", 777u64)?;
    let cfg = policy(&opt(&o, "policy", "F".to_string())?)?;
    let base = load_models(&PathBuf::from(opt(&o, "tables", "tables".to_string())?))?;
    let mut models = Models { cont: ContTable::zeros(kmax, hmax, cmax), leaf: base.leaf };
    let t0 = Instant::now();
    for k in 1..=kmax {
        let cells: Vec<(u8, u8)> = (0..=hmax).flat_map(|h| (1..=cmax).map(move |c| (h, c))).collect();
        let values: Vec<(u8, u8, f64, f64)> = cells
            .par_iter()
            .map_init(Planner::default, |planner, &(h, c)| {
                let (mut s, mut m) = (0.0, 0.0);
                for i in 0..samples {
                    let po = play_puzzle(
                        &ctx,
                        &models,
                        planner,
                        &cfg,
                        mix(&[seed, k as u64, i, 0]),
                        mix(&[seed, k as u64, i, 1]),
                        k - 1,
                        h,
                        c,
                        false,
                    );
                    m += po.matches as f64;
                    if po.solved {
                        let (fs, fm) = models.cont.get(k - 1, h - po.hints_used, c - po.challenges_used);
                        s += 1.0 + fs;
                        m += fm;
                    }
                }
                (h, c, s / samples as f64, m / samples as f64)
            })
            .collect();
        for (h, c, s, m) in values {
            models.cont.set(k, h, c, s, m);
        }
        models.cont.repair();
        let (s_full, _) = models.cont.get(k, hmax, cmax);
        let (s_mid, _) = models.cont.get(k, hmax.min(11), (4 * k).min(cmax));
        println!(
            "k={k}: F̂[k][{hmax}][{cmax}] = {s_full:.3}, F̂[k][11][{}] = {s_mid:.3}  ({:.0} s)",
            (4 * k).min(cmax),
            t0.elapsed().as_secs_f64()
        );
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, models.cont.to_bytes())?;
    println!("wrote {}", out.display());
    Ok(())
}

/// Fit the cost-to-go leaf from decisions in solved puzzles of simulated events (preset mixture).
pub fn fit_leaf(bin: &Path, args: &[&str]) -> Res {
    let o = parse_opts(args)?;
    let ctx = load_ctx(bin)?;
    let out = PathBuf::from(opt(&o, "out", "tables/leaf.bin".to_string())?);
    let models = load_models(&PathBuf::from(opt(&o, "tables", "tables".to_string())?))?;
    let cfg = policy(&opt(&o, "policy", "F".to_string())?)?;
    let presets: Vec<EventCfg> = opt(&o, "presets", "9/11/36,9/11/28,9/4/30,9/0/36".to_string())?
        .split(',')
        .map(preset)
        .collect::<Result<_, _>>()?;
    let (n, seed) = (opt(&o, "n", 4000u64)?, opt(&o, "seed", 4242u64)?);
    let t0 = Instant::now();
    let per = n / presets.len() as u64;
    let samples: Vec<(f64, bool, u8, u8)> = presets
        .iter()
        .enumerate()
        .flat_map(|(pi, &event)| {
            let batch: Vec<Vec<(f64, bool, u8, u8)>> = (0..per)
                .into_par_iter()
                .map_init(Planner::default, |planner, i| {
                    let mut s = Vec::new();
                    play_activity(
                        &ctx,
                        &models,
                        planner,
                        &cfg,
                        event,
                        mix(&[seed, pi as u64]),
                        i,
                        Some(&mut s),
                    );
                    s
                })
                .collect();
            batch.into_iter().flatten()
        })
        .collect();
    let ctg = CostToGo::fit(&samples, 40);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, ctg.to_bytes())?;
    println!(
        "{} samples from {} events → {} ({:.0} s)",
        samples.len(),
        per * presets.len() as u64,
        out.display(),
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Per-decision latency of a policy, measured single-threaded along simulated events.
pub fn bench_decide(bin: &Path, args: &[&str]) -> Res {
    use decoder_core::belief::{Belief, Event};
    use decoder_core::data::{ALL_FIELDS, Field};
    use decoder_core::policy::DecisionInput;
    use decoder_core::rng::Rng;
    use decoder_core::search::Action;
    let o = parse_opts(args)?;
    let ctx = load_ctx(bin)?;
    let models = load_models(&PathBuf::from(opt(&o, "tables", "tables".to_string())?))?;
    let cfg = policy(&opt(&o, "policy", "FV".to_string())?)?;
    let event = preset(&opt(&o, "preset", "9/11/36".to_string())?)?;
    let n = opt(&o, "n", 30u64)?;
    let mut planner = Planner::default();
    let mut times: Vec<(f64, usize)> = Vec::new();
    for i in 0..n {
        let (mut hints, mut challenges) = (event.hints, event.challenges);
        for p in 0..event.puzzles {
            let mut trng = Rng::new(mix(&[5, i, p as u64, 0]));
            let mut hrng = Rng::new(mix(&[5, i, p as u64, 1]));
            let weights: Vec<u32> = ctx.pool_groups.iter().map(|&g| ctx.weights[g as usize]).collect();
            let target = ctx.pool_groups[trng.weighted(&weights)];
            let f = Field::from_index(trng.below(6) as usize);
            let mut b = Belief::prior(&ctx);
            b.apply(&ctx, &Event::Reveal { field: f, shown: ctx.table.shown(f, target) }).unwrap();
            while !b.solved && challenges > 0 {
                let input =
                    DecisionInput { belief: &b, hints, challenges, remaining_after: event.puzzles - p - 1 };
                let t0 = Instant::now();
                let Some(d) = planner.decide(&ctx, &models, &cfg, input) else { break };
                times.push((t0.elapsed().as_secs_f64() * 1e3, b.cands.len()));
                match d.action {
                    Action::Hint => {
                        let unknown: Vec<Field> = Field::ALL
                            .into_iter()
                            .filter(|f| (!b.known & ALL_FIELDS) & f.bit() != 0)
                            .collect();
                        let f = unknown[hrng.below(unknown.len() as u64) as usize];
                        b.apply(&ctx, &Event::Reveal { field: f, shown: ctx.table.shown(f, target) })
                            .unwrap();
                        hints -= 1;
                    }
                    Action::Challenge(g) => {
                        let fb = ctx.rules.feedback(&ctx.table, g, target);
                        b.apply(&ctx, &Event::Challenge { guess: g, fb }).unwrap();
                        challenges -= 1;
                    }
                }
            }
            if !b.solved {
                break;
            }
        }
    }
    let mut ms: Vec<f64> = times.iter().map(|t| t.0).collect();
    ms.sort_by(|a, b| a.total_cmp(b));
    let q = |f: f64| ms[((ms.len() - 1) as f64 * f).round() as usize];
    let worst = times.iter().cloned().fold((0.0, 0), |a, t| if t.0 > a.0 { t } else { a });
    println!(
        "{} decisions: p50 {:.2} ms, p90 {:.2} ms, p99 {:.2} ms, max {:.2} ms (n={} candidates)",
        ms.len(),
        q(0.5),
        q(0.9),
        q(0.99),
        worst.0,
        worst.1
    );
    Ok(())
}

/// Pack tables/cont.bin + tables/leaf.bin into tables/tables.bin for the browser.
pub fn pack_tables(dir: &Path) -> Res {
    let m = load_models(dir)?;
    let bytes = decoder_core::cont::pack_tables(&m.cont, m.leaf.as_ref());
    std::fs::write(dir.join("tables.bin"), &bytes)?;
    println!("wrote {} ({} bytes)", dir.join("tables.bin").display(), bytes.len());
    Ok(())
}
