//! `mdcd data build`: rebuild the card table from pinned upstream sources (data/raw/):
//! ygocdb `cards.json` (fields + names) and YgoMaster `CardList.json` + `YdkIds.txt` (Master Duel pool).
//! Reproduces the legacy `cards-data.js` (this was the missing builder) and writes no timestamps,
//! so rebuilding the same sources gives byte-identical output.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use decoder_core::data::{CardTable, RawGroup, STAT_NONE};
use regex::Regex;
use serde_json::{Value, json};

use crate::Res;

// ygopro type bits.
const TYPE_MONSTER: u64 = 0x1;
const TYPE_NORMAL: u64 = 0x10;
const TYPE_FUSION: u64 = 0x40;
const TYPE_RITUAL: u64 = 0x80;
const TYPE_SYNCHRO: u64 = 0x2000;
const TYPE_XYZ: u64 = 0x80_0000;
const TYPE_PENDULUM: u64 = 0x100_0000;
const TYPE_LINK: u64 = 0x400_0000;

/// One official monster card after field derivation.
struct Card {
    cid: u64,
    id: u32,
    frame: u8,
    attribute: u8,
    race: u32,
    level_n: u8,
    level_nm: u16,
    atk: i16,
    def: i16,
    names: Vec<String>,
}

fn frame_of(t: u64) -> u8 {
    let base = if t & TYPE_LINK != 0 {
        32
    } else if t & TYPE_XYZ != 0 {
        16
    } else if t & TYPE_SYNCHRO != 0 {
        8
    } else if t & TYPE_FUSION != 0 {
        4
    } else if t & TYPE_RITUAL != 0 {
        64
    } else if t & TYPE_NORMAL != 0 {
        1
    } else {
        2
    };
    base | if t & TYPE_PENDULUM != 0 { 128 } else { 0 }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c) || ('\u{3040}'..='\u{30ff}').contains(&c))
}

fn read_cards(raw: &Path) -> Result<Vec<Card>, Box<dyn std::error::Error>> {
    let db: BTreeMap<String, Value> = serde_json::from_slice(&std::fs::read(raw.join("ygocdb-cards.json"))?)?;
    // 「规则上，这张卡的等级/阶级当作N星/阶使用」: the printed level differs from the rule level (printed 0).
    let rule_level = Regex::new(r"规则上，这张卡的(等级|阶级)当作\d+(星|阶)使用")?;
    let mut cards = Vec::new();
    for e in db.values() {
        let Some(d) = e.get("data") else { continue };
        let t = d["type"].as_u64().ok_or("type")?;
        let id = e["id"].as_u64().ok_or("id")?;
        if t & TYPE_MONSTER == 0 || id >= 100_000_000 {
            continue; // not a monster, or a pre-release placeholder id
        }
        let level_n = (d["level"].as_u64().ok_or("level")? & 0xff) as u8;
        let desc = e["text"]["desc"].as_str().unwrap_or("");
        let printed = if rule_level.is_match(desc) { 0 } else { level_n };
        let names: Vec<String> =
            ["md_name", "sc_name", "cn_name", "nwbbs_n", "cnocg_n", "en_name", "jp_name"]
                .iter()
                .filter_map(|k| e[*k].as_str())
                .filter(|s| !s.is_empty())
                .fold(Vec::new(), |mut v, s| {
                    if !v.iter().any(|x| x == s) {
                        v.push(s.to_string());
                    }
                    v
                });
        let is_link = t & TYPE_LINK != 0;
        cards.push(Card {
            cid: e["cid"].as_u64().ok_or("cid")?,
            id: id as u32,
            frame: frame_of(t),
            attribute: u8::try_from(d["attribute"].as_u64().ok_or("attribute")?)?,
            race: u32::try_from(d["race"].as_u64().ok_or("race")?)?,
            level_n,
            level_nm: (1u16 << level_n) | (1u16 << printed),
            atk: i16::try_from(d["atk"].as_i64().ok_or("atk")?)?,
            def: if is_link { STAT_NONE } else { i16::try_from(d["def"].as_i64().ok_or("def")?)? },
            names,
        });
    }
    let rule_cards = cards.iter().filter(|c| c.level_nm.count_ones() == 2).count();
    if rule_cards != 7 {
        return Err(format!("expected 7 rule-level cards, found {rule_cards} (check the regex)").into());
    }
    Ok(cards)
}

/// Passcodes present in Master Duel: YdkIds maps passcode → internal id, CardList lists internal ids.
fn read_md_passcodes(raw: &Path) -> Result<HashSet<u32>, Box<dyn std::error::Error>> {
    let list: HashMap<String, Value> = serde_json::from_slice(&std::fs::read(raw.join("CardList.json"))?)?;
    let internal: HashSet<u32> = list.keys().filter_map(|k| k.parse().ok()).collect();
    let mut out = HashSet::new();
    for line in std::fs::read_to_string(raw.join("YdkIds.txt"))?.lines() {
        let mut it = line.split_whitespace().map(|x| x.parse::<u32>());
        if let (Some(Ok(passcode)), Some(Ok(iid))) = (it.next(), it.next())
            && passcode > 0
            && internal.contains(&iid)
        {
            out.insert(passcode);
        }
    }
    Ok(out)
}

pub struct Built {
    pub table: CardTable,
    pub display: Vec<String>,
    pub aliases: Vec<Vec<String>>,
}

pub fn build(raw: &Path) -> Result<Built, Box<dyn std::error::Error>> {
    let mut cards = read_cards(raw)?;
    let md = read_md_passcodes(raw)?;
    cards.sort_by_key(|c| c.cid);
    type Key = (u8, u8, u32, u8, u16, i16, i16);
    let mut groups: BTreeMap<Key, Vec<&Card>> = BTreeMap::new();
    for c in &cards {
        groups
            .entry((c.frame, c.attribute, c.race, c.level_n, c.level_nm, c.atk, c.def))
            .or_default()
            .push(c);
    }
    let mut list: Vec<(RawGroup, String, Vec<String>)> = groups
        .values()
        .map(|members| {
            let c0 = members[0];
            let ids: Vec<u32> = members.iter().map(|c| c.id).collect();
            let wm = ids.iter().filter(|id| md.contains(id)).count();
            let rep = ids.iter().copied().find(|id| md.contains(id)).unwrap_or(ids[0]);
            let rep_card = members.iter().find(|c| c.id == rep).unwrap();
            let mut aliases: Vec<String> = Vec::new();
            for c in members {
                for n in &c.names {
                    if !aliases.contains(n) {
                        aliases.push(n.clone());
                    }
                }
            }
            // Display name: the representative's first CJK alias (md_name, sc_name, …), else its first name.
            let display = rep_card
                .names
                .iter()
                .find(|n| has_cjk(n))
                .or(rep_card.names.first())
                .cloned()
                .unwrap_or_default();
            let raw = RawGroup {
                frame: c0.frame,
                attribute: c0.attribute,
                race: c0.race,
                level_n: c0.level_n,
                level_nm: c0.level_nm,
                atk: c0.atk,
                def: c0.def,
                wm: wm as u8,
                wa: ids.len() as u8,
                passcodes: ids,
                rep_passcode: rep,
            };
            (raw, display, aliases)
        })
        .collect();
    // Deterministic order: Master Duel groups first, heavier groups first, then lowest passcode.
    list.sort_by_key(|(g, _, _)| {
        (g.wm == 0, std::cmp::Reverse(g.wm), std::cmp::Reverse(g.wa), *g.passcodes.iter().min().unwrap())
    });
    let raw: Vec<RawGroup> = list.iter().map(|(g, _, _)| g.clone()).collect();
    let table = CardTable::from_raw(&raw)?;
    Ok(Built {
        table,
        display: list.iter().map(|(_, d, _)| d.clone()).collect(),
        aliases: list.into_iter().map(|(_, _, a)| a).collect(),
    })
}

/// Invariants that must hold for any rebuild (fail loudly on upstream surprises).
pub fn verify(t: &CardTable) -> Result<Vec<String>, String> {
    let mut report = Vec::new();
    let d = &t.dicts;
    let err = |m: String| Err::<(), String>(m);
    let known_frames = [1u8, 2, 4, 8, 16, 32, 64, 129, 130, 132, 136, 144, 192];
    if let Some(f) = d.frame.iter().find(|f| !known_frames.contains(f)) {
        err(format!("unknown frame {f}"))?;
    }
    if d.attribute.iter().any(|a| a.count_ones() != 1) || d.race.iter().any(|r| r.count_ones() != 1) {
        err("attribute/race must be single bits".into())?;
    }
    for g in 0..t.len() {
        let is_link = d.frame[t.frame[g] as usize] == 32;
        let def = d.def[t.def[g] as usize];
        if is_link != (def == STAT_NONE) {
            err(format!("group {g}: link ⇔ DEF none violated"))?;
        }
        let atk = d.atk[t.atk[g] as usize];
        if !(atk == -2 || (0..=5000).contains(&atk)) || !(def == -2 || def == -3 || (0..=5000).contains(&def))
        {
            err(format!("group {g}: ATK/DEF out of range"))?;
        }
        if t.wm[g] > t.wa[g] {
            err(format!("group {g}: wm > wa"))?;
        }
        let pcs = t.passcodes(g as u16);
        if t.wa[g] as usize != pcs.len() || !pcs.contains(&t.rep_passcode[g]) {
            err(format!("group {g}: passcode list inconsistent"))?;
        }
    }
    if d.level_nm.iter().any(|nm| nm.count_ones() > 2) {
        err("level nm with more than 2 bits".into())?;
    }
    let mut all: Vec<u32> = t.ids.clone();
    all.sort_unstable();
    if all.windows(2).any(|w| w[0] == w[1]) {
        err("duplicate passcode".into())?;
    }
    let md_weight: u32 = t.wm.iter().map(|&w| w as u32).sum();
    let all_weight: u32 = t.wa.iter().map(|&w| w as u32).sum();
    report.push(format!(
        "groups {} (Master Duel {}), cards {} (Master Duel {}), data hash {:016x}",
        t.len(),
        t.n_md,
        all_weight,
        md_weight,
        t.data_hash
    ));
    Ok(report)
}

pub fn run_build(raw: &Path, out: &Path) -> Res {
    let built = build(raw)?;
    for line in verify(&built.table)? {
        println!("{line}");
    }
    write_outputs(&built, out, "ygocdb + YgoMaster (data/sources.lock.json)")?;
    Ok(())
}

pub fn write_outputs(built: &Built, out: &Path, source: &str) -> Res {
    let t = &built.table;
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("cards.bin"), t.to_bytes())?;
    std::fs::write(
        out.join("names.json"),
        serde_json::to_string(&json!({ "display": built.display, "aliases": built.aliases }))?,
    )?;
    let summary = json!({
        "dataHash": format!("{:016x}", t.data_hash),
        "source": source,
        "groups": t.len(),
        "mdGroups": t.n_md,
        "cards": t.wa.iter().map(|&w| w as u32).sum::<u32>(),
        "mdCards": t.wm.iter().map(|&w| w as u32).sum::<u32>(),
    });
    std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary)? + "\n")?;
    Ok(())
}

/// Compare a table with the legacy `cards-data.js` group by group (matched via passcodes).
pub fn compare_legacy(legacy: &Path, bin: &Path) -> Res {
    let groups = crate::legacy::read_groups(legacy)?;
    let t = CardTable::from_bytes(&std::fs::read(bin)?)?;
    let mut by_passcode: HashMap<u32, u16> = HashMap::new();
    for g in 0..t.len() as u16 {
        for &p in t.passcodes(g) {
            by_passcode.insert(p, g);
        }
    }
    let (mut field_mm, mut wm_mm, mut ids_mm, mut missing) = (0, 0, 0, 0);
    for lg in &groups {
        let Some(&g) = by_passcode.get(&lg.ids[0]) else {
            missing += 1;
            continue;
        };
        let gu = g as usize;
        let d = &t.dicts;
        let same = d.frame[t.frame[gu] as usize] as u32 == lg.b
            && d.attribute[t.attribute[gu] as usize] as u32 == lg.a
            && d.race[t.race[gu] as usize] == lg.r
            && t.level_n[gu] as u32 == lg.n
            && d.level_nm[t.level[gu] as usize] as u32 == lg.nm
            && d.atk[t.atk[gu] as usize] as i64 == lg.atk
            && d.def[t.def[gu] as usize] as i64 == lg.def;
        if !same {
            field_mm += 1;
            if field_mm <= 5 {
                eprintln!("field mismatch: {}", lg.name);
            }
        }
        if t.wm[gu] as u32 != lg.wm {
            wm_mm += 1;
            if wm_mm <= 5 {
                eprintln!("wm mismatch: {} legacy {} new {}", lg.name, lg.wm, t.wm[gu]);
            }
        }
        let mut a: Vec<u32> = lg.ids.clone();
        let mut b: Vec<u32> = t.passcodes(g).to_vec();
        a.sort_unstable();
        b.sort_unstable();
        if a != b {
            ids_mm += 1;
        }
    }
    println!(
        "legacy groups {}, new groups {}: missing {missing}, field mismatches {field_mm}, wm mismatches {wm_mm}, passcode-set mismatches {ids_mm}",
        groups.len(),
        t.len()
    );
    if missing + field_mm + ids_mm > 0 { Err("legacy comparison failed".into()) } else { Ok(()) }
}
