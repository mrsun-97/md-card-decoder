//! Bootstrap conversion of the legacy `legacy/cards-data.js` into `cards.bin` + name files.
//! The reproducible upstream pipeline (ygocdb + YgoMaster) replaces this in M2.

use std::collections::HashSet;
use std::path::Path;

use decoder_core::data::{CardTable, RawGroup};
use serde_json::{Value, json};

pub struct LegacyGroup {
    pub b: u32,
    pub a: u32,
    pub r: u32,
    pub n: u32,
    pub nm: u32,
    pub atk: i64,
    pub def: i64,
    pub wm: u32,
    pub wa: u32,
    pub ids: Vec<u32>,
    pub names: Vec<String>,
    pub name: String,
}

pub fn read_json(src: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(src)?;
    let body = text
        .trim()
        .strip_prefix("window.CARD_DATA=")
        .ok_or("missing window.CARD_DATA= prefix")?
        .trim_end_matches(';');
    Ok(serde_json::from_str(body)?)
}

pub fn read_groups(src: &Path) -> Result<Vec<LegacyGroup>, Box<dyn std::error::Error>> {
    let data = read_json(src)?;
    let cards = data["cards"].as_array().ok_or("cards missing")?;
    let u = |v: &Value, k: &str| -> Result<u32, String> {
        v[k].as_u64().map(|x| x as u32).ok_or_else(|| format!("field {k} missing"))
    };
    let i = |v: &Value, k: &str| -> Result<i64, String> {
        v[k].as_i64().ok_or_else(|| format!("field {k} missing"))
    };
    cards
        .iter()
        .map(|c| {
            Ok(LegacyGroup {
                b: u(c, "b")?,
                a: u(c, "a")?,
                r: u(c, "r")?,
                n: u(c, "n")?,
                nm: u(c, "nm")?,
                atk: i(c, "atk")?,
                def: i(c, "def")?,
                wm: u(c, "wm")?,
                wa: u(c, "wa")?,
                ids: c["ids"].as_array().ok_or("ids")?.iter().map(|x| x.as_u64().unwrap() as u32).collect(),
                names: c["names"]
                    .as_array()
                    .ok_or("names")?
                    .iter()
                    .map(|x| x.as_str().unwrap().to_string())
                    .collect(),
                name: c["name"].as_str().ok_or("name")?.to_string(),
            })
        })
        .collect()
}

pub fn convert(src: &Path, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data = read_json(src)?;
    let groups = read_groups(src)?;
    // Deterministic order: Master Duel groups first, heavier groups first, then lowest passcode.
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by_key(|&i| {
        let g = &groups[i];
        (g.wm == 0, std::cmp::Reverse(g.wm), std::cmp::Reverse(g.wa), *g.ids.iter().min().unwrap())
    });
    let mut keys = HashSet::new();
    let mut raw = Vec::with_capacity(groups.len());
    for &i in &order {
        let g = &groups[i];
        if !keys.insert((g.b, g.a, g.r, g.n, g.nm, g.atk, g.def)) {
            return Err(format!("duplicate group key for {}", g.name).into());
        }
        if g.wm > 255 || g.wa > 255 || g.wm > g.wa {
            return Err(format!("bad weights for {}", g.name).into());
        }
        raw.push(RawGroup {
            frame: u8::try_from(g.b)?,
            attribute: u8::try_from(g.a)?,
            race: g.r,
            level_n: u8::try_from(g.n)?,
            level_nm: u16::try_from(g.nm)?,
            atk: i16::try_from(g.atk)?,
            def: i16::try_from(g.def)?,
            wm: g.wm as u8,
            wa: g.wa as u8,
            passcodes: g.ids.clone(),
            // The legacy file does not record which passcode is in Master Duel; M2 fixes this.
            rep_passcode: g.ids[0],
        });
    }
    let table = CardTable::from_raw(&raw)?;
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("cards.bin"), table.to_bytes())?;
    let names: Vec<Value> = order.iter().map(|&i| json!(groups[i].names)).collect();
    let display: Vec<&str> = order.iter().map(|&i| groups[i].name.as_str()).collect();
    let mut legacy_to_new = vec![0u32; groups.len()];
    for (new, &old) in order.iter().enumerate() {
        legacy_to_new[old] = new as u32;
    }
    let meta = json!({
        "dataHash": format!("{:016x}", table.data_hash),
        "source": "legacy legacy/cards-data.js",
        "legacyGeneratedAt": data["generatedAt"],
        "labels": data["labels"],
        "groups": table.len(),
        "mdGroups": table.n_md,
    });
    std::fs::write(
        out.join("names.json"),
        serde_json::to_string(&json!({ "display": display, "aliases": names }))?,
    )?;
    std::fs::write(out.join("legacy-index-map.json"), serde_json::to_string(&legacy_to_new)?)?;
    std::fs::write(out.join("meta.json"), serde_json::to_string_pretty(&meta)?)?;
    println!(
        "wrote {} groups ({} Master Duel), data hash {:016x}, cards.bin {} bytes",
        table.len(),
        table.n_md,
        table.data_hash,
        table.to_bytes().len()
    );
    Ok(())
}
