//! Official rule examples and real-play logs (tests/golden/rules.toml) against the built card table.
//! Needs `npm run data:build` (data/generated/cards.bin); skipped otherwise.

use std::path::PathBuf;

use decoder_core::{CardTable, Field, RulePolicy, Rules};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixtures {
    judge: Vec<Judge>,
    replay: Vec<Replay>,
}

#[derive(Deserialize)]
struct Judge {
    name: String,
    target: u32,
    guess: u32,
    matched: Matched,
    frame_reveal: Option<i64>,
    level_reveal: Option<u8>,
}

#[derive(Deserialize)]
struct Replay {
    name: String,
    target: u32,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Step {
    Challenge {
        guess: u32,
        matched: Matched,
    },
    Reveal {
        reveal: String,
        value: i64,
        #[allow(dead_code)]
        source: String,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Matched {
    All(String),
    Fields(Vec<String>),
}

fn field(name: &str) -> Field {
    match name {
        "frame" => Field::Frame,
        "attribute" => Field::Attribute,
        "race" => Field::Race,
        "level" => Field::Level,
        "atk" => Field::Atk,
        "def" => Field::Def,
        other => panic!("unknown field {other}"),
    }
}

fn mask(m: &Matched) -> u8 {
    match m {
        Matched::All(s) => {
            assert_eq!(s, "all");
            63
        }
        Matched::Fields(f) => f.iter().fold(0, |acc, n| acc | field(n).bit()),
    }
}

fn load() -> Option<(CardTable, Fixtures)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = std::fs::read(root.join("data/generated/cards.bin")).ok()?;
    let fixtures =
        toml::from_str(&std::fs::read_to_string(root.join("tests/golden/rules.toml")).unwrap()).unwrap();
    Some((CardTable::from_bytes(&bin).unwrap(), fixtures))
}

#[test]
fn official_examples_and_real_play() {
    let Some((t, fx)) = load() else {
        eprintln!("skipped: data/generated/cards.bin missing (run npm run data:build)");
        return;
    };
    let rules = Rules::new(&t, RulePolicy::default());
    let gid = |pc: u32| t.group_of_passcode(pc).unwrap_or_else(|| panic!("passcode {pc} not in data"));
    for j in &fx.judge {
        let fb = rules.feedback(&t, gid(j.guess), gid(j.target));
        assert_eq!(fb.mask(), mask(&j.matched), "{}", j.name);
        if let Some(f) = j.frame_reveal {
            assert_eq!(fb.frame_reveal().map(|id| t.dicts.frame[id as usize] as i64), Some(f), "{}", j.name);
        }
        if let Some(n) = j.level_reveal {
            assert_eq!(fb.level_reveal(), Some(n), "{}", j.name);
        }
    }
    for r in &fx.replay {
        let target = gid(r.target);
        for (i, step) in r.steps.iter().enumerate() {
            match step {
                Step::Challenge { guess, matched } => {
                    let fb = rules.feedback(&t, gid(*guess), target);
                    assert_eq!(fb.mask(), mask(matched), "{} step {i}", r.name);
                }
                Step::Reveal { reveal, value, .. } => {
                    let f = field(reveal);
                    assert_eq!(t.shown_raw(f, t.shown(f, target)), *value, "{} step {i}", r.name);
                }
            }
        }
    }
}
