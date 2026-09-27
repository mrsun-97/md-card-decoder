//! WebAssembly facade. Kept thin: all logic lives in `decoder-core`; this file only converts
//! between JSON (the page's state format) and core types.
//!
//! Cards are identified by group index (`gid`, position in cards.bin); field values cross the
//! boundary as the raw values the game shows (frame bits, attribute bit, race bit, rule level,
//! ATK, DEF). Field order: frame, attribute, race, level, atk, def.

use decoder_core::belief::{Belief, Event, relax1};
use decoder_core::cont::unpack_tables;
use decoder_core::kernel::{Cell, Partitioner};
use decoder_core::policy::{DecisionInput, Method, Models, Planner, PolicyCfg};
use decoder_core::recommend::recommend;
use decoder_core::rules::Fb;
use decoder_core::search::Action;
use decoder_core::sim::{EventCfg, play_activity};
use decoder_core::{CardTable, Ctx, Field, Gid, Pool, RulePolicy, Val};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Engine {
    table_bytes: Vec<u8>,
    ctx: Ctx,
    models: Models,
    cfg: PolicyCfg,
    planner: Planner,
    part: Partitioner,
}

fn err(msg: impl std::fmt::Display) -> JsError {
    JsError::new(&msg.to_string())
}

fn parse(json: &str) -> Result<Value, JsError> {
    serde_json::from_str(json).map_err(|e| err(format!("bad JSON: {e}")))
}

fn val(v: Val) -> Value {
    json!([v.0[0], v.0[1], v.0[2]])
}

fn action_json(a: Action) -> Value {
    match a {
        Action::Hint => json!({ "type": "hint" }),
        Action::Challenge(g) => json!({ "type": "challenge", "gid": g }),
    }
}

#[wasm_bindgen]
impl Engine {
    /// `cards_bin`: cards.bin; `tables_bin`: packed fitted models (may be empty → legacy formula).
    #[wasm_bindgen(constructor)]
    pub fn new(cards_bin: &[u8], tables_bin: &[u8]) -> Result<Engine, JsError> {
        let table = CardTable::from_bytes(cards_bin).map_err(err)?;
        let models = if tables_bin.is_empty() {
            Models::legacy_only()
        } else {
            let (cont, leaf) = unpack_tables(tables_bin).map_err(err)?;
            Models { cont, leaf }
        };
        let ctx = Ctx::new(table, RulePolicy::default(), Pool::MasterDuel);
        let cfg = if models.leaf.is_some() { PolicyCfg::fv2() } else { PolicyCfg::fitted_zero_leaf() };
        Ok(Engine {
            table_bytes: cards_bin.to_vec(),
            ctx,
            models,
            cfg,
            planner: Planner::default(),
            part: Partitioner::default(),
        })
    }

    #[wasm_bindgen(getter)]
    pub fn groups(&self) -> u32 {
        self.ctx.table.len() as u32
    }

    #[wasm_bindgen(getter, js_name = mdGroups)]
    pub fn md_groups(&self) -> u32 {
        self.ctx.table.n_md as u32
    }

    #[wasm_bindgen(getter, js_name = dataHash)]
    pub fn data_hash(&self) -> String {
        format!("{:016x}", self.ctx.table.data_hash)
    }

    /// Settings: `{"pool":"md"|"all","unknownStatMatches":bool,"linkDefMatches":bool,
    /// "consistentOnly":bool,"exact":bool,"extraEntropy":n}` (all optional).
    #[wasm_bindgen(js_name = setPolicy)]
    pub fn set_policy(&mut self, json: &str) -> Result<(), JsError> {
        let o = parse(json)?;
        let pool = match o["pool"].as_str() {
            Some("all") => Pool::All,
            _ => Pool::MasterDuel,
        };
        let rules = RulePolicy {
            unknown_stat_matches: o["unknownStatMatches"].as_bool().unwrap_or(true),
            link_def_matches: o["linkDefMatches"].as_bool().unwrap_or(true),
        };
        if pool != self.ctx.pool || rules != self.ctx.rules.policy {
            let table = CardTable::from_bytes(&self.table_bytes).map_err(err)?;
            self.ctx = Ctx::new(table, rules, pool);
        }
        self.cfg.consistent_only = o["consistentOnly"].as_bool().unwrap_or(false);
        if let Some(x) = o["exact"].as_bool() {
            self.cfg.exact = x;
        }
        if let Some(k) = o["extraEntropy"].as_u64() {
            self.cfg.extra_entropy = k as usize;
        }
        Ok(())
    }

    /// Raw field values of every group, for rendering: `{frame:[..], attribute:[..], …, wm, wa, passcode}`.
    #[wasm_bindgen(js_name = columns)]
    pub fn columns(&self) -> String {
        let t = &self.ctx.table;
        let col =
            |f: Field| -> Vec<i64> { (0..t.len() as Gid).map(|g| t.shown_raw(f, t.shown(f, g))).collect() };
        json!({
            "frame": col(Field::Frame),
            "attribute": col(Field::Attribute),
            "race": col(Field::Race),
            "level": col(Field::Level),
            "atk": col(Field::Atk),
            "def": col(Field::Def),
            "levelMask": (0..t.len()).map(|g| t.dicts.level_nm[t.level[g] as usize]).collect::<Vec<_>>(),
            "wm": t.wm,
            "wa": t.wa,
            "passcode": t.rep_passcode,
        })
        .to_string()
    }

    fn events(&self, state: &Value) -> Result<Vec<Event>, JsError> {
        let t = &self.ctx.table;
        let mut out = Vec::new();
        for e in state["events"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(r) = e.get("reveal") {
                let fi = r["field"].as_u64().ok_or_else(|| err("reveal.field"))? as usize;
                let field = *Field::ALL.get(fi).ok_or_else(|| err("reveal.field out of range"))?;
                let raw = r["value"].as_i64().ok_or_else(|| err("reveal.value"))?;
                let shown = t
                    .shown_from_raw(field, raw)
                    .ok_or_else(|| err(format!("unknown value {raw} for field {fi}")))?;
                out.push(if r["matched"].as_bool().unwrap_or(false) {
                    Event::Matched { field, shown }
                } else {
                    Event::Reveal { field, shown }
                });
            } else if let Some(c) = e.get("challenge") {
                let guess = c["guess"].as_u64().ok_or_else(|| err("challenge.guess"))? as Gid;
                if guess as usize >= t.len() {
                    return Err(err("challenge.guess out of range"));
                }
                let mask = c["mask"].as_u64().ok_or_else(|| err("challenge.mask"))? as u8;
                let frame = c["frame"].as_i64().and_then(|raw| t.shown_from_raw(Field::Frame, raw));
                let level = c["level"].as_u64().map(|n| n as u8);
                let fb =
                    Fb::from_parts(mask, frame, level).ok_or_else(|| err("challenge feedback incomplete"))?;
                out.push(Event::Challenge { guess, fb });
            } else {
                return Err(err("event must be reveal or challenge"));
            }
        }
        Ok(out)
    }

    fn belief_or_contradiction(&self, events: &[Event]) -> Result<Belief, Value> {
        Belief::from_events(&self.ctx, events).map_err(|c| {
            let suggestions: Vec<Value> =
                relax1(&self.ctx, events).into_iter().take(3).map(|(i, m)| json!([i, m])).collect();
            json!({ "ok": false, "contradiction": { "event": c.event, "suggestions": suggestions } })
        })
    }

    /// Candidate set and what is known: `{ok, count, mass, entropy, candidates:[gid], top:[[gid,p]],
    /// known:[raw|null ×6], matched, solved}` or `{ok:false, contradiction:{event, suggestions}}`.
    pub fn belief(&self, state_json: &str) -> Result<String, JsError> {
        let state = parse(state_json)?;
        let events = self.events(&state)?;
        let b = match self.belief_or_contradiction(&events) {
            Ok(b) => b,
            Err(v) => return Ok(v.to_string()),
        };
        let t = &self.ctx.table;
        let mass = self.ctx.mass(&b.cands);
        let mut top: Vec<(Gid, u32)> = b.cands.iter().map(|&g| (g, self.ctx.weights[g as usize])).collect();
        top.sort_by(|a, x| x.1.cmp(&a.1).then(a.0.cmp(&x.0)));
        let known: Vec<Value> = Field::ALL
            .iter()
            .map(|&f| b.values[f as usize].map_or(Value::Null, |s| json!(t.shown_raw(f, s))))
            .collect();
        Ok(json!({
            "ok": true,
            "count": b.cands.len(),
            "mass": mass,
            "entropy": self.ctx.entropy(&b.cands),
            "candidates": b.cands,
            "top": top.iter().take(10).map(|&(g, w)| json!([g, w as f64 / mass as f64])).collect::<Vec<_>>(),
            "known": known,
            "matched": b.matched,
            "solved": b.solved,
        })
        .to_string())
    }

    /// Values a field can still show, with candidate mass and count: `[[raw, mass, count], …]`.
    #[wasm_bindgen(js_name = revealOptions)]
    pub fn reveal_options(&self, state_json: &str, field: u8) -> Result<String, JsError> {
        let state = parse(state_json)?;
        let events = self.events(&state)?;
        let f = *Field::ALL.get(field as usize).ok_or_else(|| err("field out of range"))?;
        let b = self.belief_or_contradiction(&events).map_err(|_| err("state is contradictory"))?;
        let t = &self.ctx.table;
        let mut acc: Vec<(i64, u64, u32)> = Vec::new();
        for &g in &b.cands {
            let raw = t.shown_raw(f, t.shown(f, g));
            match acc.iter_mut().find(|x| x.0 == raw) {
                Some(x) => {
                    x.1 += self.ctx.weights[g as usize] as u64;
                    x.2 += 1;
                }
                None => acc.push((raw, self.ctx.weights[g as usize] as u64, 1)),
            }
        }
        acc.sort_by(|a, x| x.1.cmp(&a.1).then(a.0.cmp(&x.0)));
        Ok(serde_json::to_string(&acc).unwrap())
    }

    /// How the game would judge `guess` against `target`: `{mask, frame, level, solved}`.
    pub fn judge(&self, guess: u32, target: u32) -> Result<String, JsError> {
        let t = &self.ctx.table;
        if guess as usize >= t.len() || target as usize >= t.len() {
            return Err(err("gid out of range"));
        }
        let fb = self.ctx.rules.feedback(t, guess as Gid, target as Gid);
        Ok(json!({
            "mask": fb.mask(),
            "frame": fb.frame_reveal().map(|f| t.dicts.frame[f as usize]),
            "level": fb.level_reveal(),
            "solved": fb.solved(),
        })
        .to_string())
    }

    /// Recommend the next action. State: `{events, hints, challenges, remainingAfter}` where
    /// hints/challenges are the *effective* inventory (current + grants still to come).
    pub fn recommend(&mut self, state_json: &str) -> Result<String, JsError> {
        let state = parse(state_json)?;
        let events = self.events(&state)?;
        let b = match self.belief_or_contradiction(&events) {
            Ok(b) => b,
            Err(v) => return Ok(v.to_string()),
        };
        let clamp = |k: &str| state[k].as_u64().unwrap_or(0).min(255) as u8;
        let input = DecisionInput {
            belief: &b,
            hints: clamp("hints"),
            challenges: clamp("challenges"),
            remaining_after: clamp("remainingAfter"),
        };
        let Some(r) = recommend(&self.ctx, &self.models, &mut self.planner, &self.cfg, input, 12) else {
            return Ok(json!({ "ok": true, "action": null }).to_string());
        };
        let (method, depth) = match r.decision.method {
            Method::ExactFinal => ("exact-final", 0),
            Method::ExactInPuzzle => ("exact-in-puzzle", 0),
            Method::Horizon { depth } => ("horizon", depth),
        };
        let actions: Vec<Value> = r
            .actions
            .iter()
            .map(|s| {
                let mut a = action_json(s.action);
                a["value"] = val(s.value);
                a["pSolve"] = json!(s.p_solve);
                a["eNewMatches"] = json!(s.e_new_matches);
                a["entropy"] = json!(s.entropy);
                a["outcomes"] = json!(s.outcomes);
                a
            })
            .collect();
        Ok(json!({
            "ok": true,
            "action": action_json(r.decision.action),
            "value": val(r.decision.value),
            "method": method,
            "depth": depth,
            "rootActions": r.decision.root_actions,
            "actions": actions,
            "hintValue": r.hint_value.map(val),
            "bestChallengeValue": r.best_challenge_value.map(val),
            "ties": r.ties.into_iter().map(action_json).collect::<Vec<_>>(),
            "count": r.candidates,
            "mass": r.mass,
            "entropy": r.entropy,
            "top": r.top.iter().map(|&(g, p)| json!([g, p])).collect::<Vec<_>>(),
            "forecast": r.forecast.map(|f| json!({ "pSolvePuzzle": f.p_solve_puzzle, "eChallenges": f.e_challenges, "eHints": f.e_hints })),
        })
        .to_string())
    }

    /// Simulate activities `first..first+count` with seed (hi, lo): `{solvedHist:[..], hints, challenges}`.
    #[wasm_bindgen(js_name = simulateBatch)]
    pub fn simulate_batch(
        &mut self,
        preset_json: &str,
        seed_hi: u32,
        seed_lo: u32,
        first: u32,
        count: u32,
    ) -> Result<String, JsError> {
        let p = parse(preset_json)?;
        let get = |k: &str| p[k].as_u64().unwrap_or(0).min(255) as u8;
        let event = EventCfg { puzzles: get("puzzles"), hints: get("hints"), challenges: get("challenges") };
        let seed = (seed_hi as u64) << 32 | seed_lo as u64;
        let mut hist = vec![0u32; event.puzzles as usize + 1];
        let (mut hints, mut challenges, mut matches) = (0u64, 0u64, 0u64);
        for i in first..first + count {
            let o = play_activity(
                &self.ctx,
                &self.models,
                &mut self.planner,
                &self.cfg,
                event,
                seed,
                i as u64,
                None,
            );
            hist[o.solved as usize] += 1;
            hints += o.hints_used as u64;
            challenges += o.challenges_used as u64;
            matches += o.matches as u64;
        }
        Ok(json!({ "solvedHist": hist, "hints": hints, "challenges": challenges, "matches": matches })
            .to_string())
    }

    /// Benchmark kernel (M1 spike): partition `cands` by every pool guess; returns the cell total.
    #[wasm_bindgen(js_name = benchPartitionAll)]
    pub fn bench_partition_all(&mut self, cands: &[u16]) -> u32 {
        let mut cells: Vec<Cell> = Vec::with_capacity(64);
        let mut total = 0u32;
        for i in 0..self.ctx.pool_groups.len() {
            let g: Gid = self.ctx.pool_groups[i];
            self.part.cells(&self.ctx, g, cands, &mut cells);
            total += cells.len() as u32;
        }
        total
    }
}
