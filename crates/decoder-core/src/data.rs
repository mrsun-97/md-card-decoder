//! Card table: one row per behaviour group (cards whose six judged fields behave identically),
//! stored column-wise with small per-field value dictionaries so every field fits in a `u8`.
//!
//! Binary format `cards.bin` (little endian), produced by `mdcd data`:
//!
//! ```text
//! magic "MDCD" | version u16 | reserved u16 | n_groups u32 | n_md u32 | data_hash u64
//! dictionaries, each `count u16` followed by the raw values:
//!   frame u8 · attribute u8 · race u32 · level nm u16 · atk i16 · def i16
//! columns, n_groups bytes each:
//!   frame · attribute · race · level (nm id) · level_n (rule level) · atk · def · wm · wa
//! representative passcode u32 × n_groups
//! passcode lists: offsets u32 × (n_groups + 1), passcodes u32 × total
//! ```
//!
//! `data_hash` is FNV-1a 64 over everything after the header; saved games refer to it.

use std::fmt;

pub const MAGIC: &[u8; 4] = b"MDCD";
pub const FORMAT_VERSION: u16 = 1;

/// Sentinel for '?' ATK/DEF.
pub const STAT_UNKNOWN: i16 = -2;
/// Sentinel for the DEF of a Link monster (it has none).
pub const STAT_NONE: i16 = -3;

/// Raw frame bits as used by the source data.
pub mod frame_bits {
    pub const NORMAL: u8 = 1;
    pub const EFFECT: u8 = 2;
    pub const FUSION: u8 = 4;
    pub const SYNCHRO: u8 = 8;
    pub const XYZ: u8 = 16;
    pub const LINK: u8 = 32;
    pub const RITUAL: u8 = 64;
    pub const PENDULUM: u8 = 128;
}

/// Group index into the table.
pub type Gid = u16;

/// The six judged fields, in match-mask bit order (bit i = field i).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Field {
    Frame = 0,
    Attribute = 1,
    Race = 2,
    Level = 3,
    Atk = 4,
    Def = 5,
}

impl Field {
    pub const ALL: [Field; 6] =
        [Field::Frame, Field::Attribute, Field::Race, Field::Level, Field::Atk, Field::Def];

    #[inline]
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }

    #[inline]
    pub fn from_index(i: usize) -> Field {
        Field::ALL[i]
    }
}

/// Mask with all six fields set.
pub const ALL_FIELDS: u8 = 0b11_1111;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dicts {
    pub frame: Vec<u8>,
    pub attribute: Vec<u8>,
    pub race: Vec<u32>,
    /// Level match masks: bit k set = level/rank/link k matches (printed and rule level).
    pub level_nm: Vec<u16>,
    pub atk: Vec<i16>,
    pub def: Vec<i16>,
}

impl Dicts {
    pub fn len(&self, field: Field) -> usize {
        match field {
            Field::Frame => self.frame.len(),
            Field::Attribute => self.attribute.len(),
            Field::Race => self.race.len(),
            Field::Level => self.level_nm.len(),
            Field::Atk => self.atk.len(),
            Field::Def => self.def.len(),
        }
    }
}

/// Column-wise card table. Groups `0..n_md` are the Master Duel pool (weight `wm > 0`).
#[derive(Clone, Debug, Default)]
pub struct CardTable {
    pub dicts: Dicts,
    pub frame: Vec<u8>,
    pub attribute: Vec<u8>,
    pub race: Vec<u8>,
    /// Level match id (index into `dicts.level_nm`).
    pub level: Vec<u8>,
    /// Rule level/rank/link rating (1..=13); what hints and level reveals show.
    pub level_n: Vec<u8>,
    pub atk: Vec<u8>,
    pub def: Vec<u8>,
    pub wm: Vec<u8>,
    pub wa: Vec<u8>,
    pub rep_passcode: Vec<u32>,
    pub ids_off: Vec<u32>,
    pub ids: Vec<u32>,
    pub n_md: usize,
    pub data_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    BadMagic,
    UnsupportedVersion(u16),
    Truncated,
    Invalid(String),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::BadMagic => write!(f, "not a cards.bin file"),
            DataError::UnsupportedVersion(v) => write!(f, "unsupported cards.bin version {v}"),
            DataError::Truncated => write!(f, "cards.bin is truncated"),
            DataError::Invalid(msg) => write!(f, "invalid cards.bin: {msg}"),
        }
    }
}

impl std::error::Error for DataError {}

/// Raw per-group values used to build a table (the builder input).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawGroup {
    pub frame: u8,
    pub attribute: u8,
    pub race: u32,
    pub level_n: u8,
    pub level_nm: u16,
    pub atk: i16,
    pub def: i16,
    pub wm: u8,
    pub wa: u8,
    pub passcodes: Vec<u32>,
    /// Representative passcode (first Master Duel passcode if any).
    pub rep_passcode: u32,
}

impl CardTable {
    #[inline]
    pub fn len(&self) -> usize {
        self.frame.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.frame.is_empty()
    }

    /// Value id of `field` for group `g`. For the level field this is the *match* id (nm);
    /// use [`CardTable::shown`] for what the game displays.
    #[inline]
    pub fn id(&self, field: Field, g: Gid) -> u8 {
        let g = g as usize;
        match field {
            Field::Frame => self.frame[g],
            Field::Attribute => self.attribute[g],
            Field::Race => self.race[g],
            Field::Level => self.level[g],
            Field::Atk => self.atk[g],
            Field::Def => self.def[g],
        }
    }

    /// The value the game shows for `field` (hints, reveals, known fields): the value id for
    /// every field except level, where it is the rule level `n`.
    #[inline]
    pub fn shown(&self, field: Field, g: Gid) -> u8 {
        match field {
            Field::Level => self.level_n[g as usize],
            _ => self.id(field, g),
        }
    }

    /// Number of distinct shown values of `field` (upper bound of `shown` + 1).
    pub fn shown_radix(&self, field: Field) -> usize {
        match field {
            Field::Level => 16,
            _ => self.dicts.len(field),
        }
    }

    /// Raw display value of a shown value (frame bits, attribute bit, race bit, level n, ATK, DEF).
    pub fn shown_raw(&self, field: Field, shown: u8) -> i64 {
        let s = shown as usize;
        match field {
            Field::Frame => self.dicts.frame[s] as i64,
            Field::Attribute => self.dicts.attribute[s] as i64,
            Field::Race => self.dicts.race[s] as i64,
            Field::Level => shown as i64,
            Field::Atk => self.dicts.atk[s] as i64,
            Field::Def => self.dicts.def[s] as i64,
        }
    }

    /// Shown value id for a raw display value, if it exists in the dictionary.
    pub fn shown_from_raw(&self, field: Field, raw: i64) -> Option<u8> {
        let pos = match field {
            Field::Frame => self.dicts.frame.iter().position(|&v| v as i64 == raw),
            Field::Attribute => self.dicts.attribute.iter().position(|&v| v as i64 == raw),
            Field::Race => self.dicts.race.iter().position(|&v| v as i64 == raw),
            Field::Level => return (1..=15).contains(&raw).then_some(raw as u8),
            Field::Atk => self.dicts.atk.iter().position(|&v| v as i64 == raw),
            Field::Def => self.dicts.def.iter().position(|&v| v as i64 == raw),
        };
        pos.map(|p| p as u8)
    }

    #[inline]
    pub fn weight(&self, pool: Pool, g: Gid) -> u32 {
        match pool {
            Pool::MasterDuel => self.wm[g as usize] as u32,
            Pool::All => self.wa[g as usize] as u32,
        }
    }

    pub fn passcodes(&self, g: Gid) -> &[u32] {
        let g = g as usize;
        &self.ids[self.ids_off[g] as usize..self.ids_off[g + 1] as usize]
    }

    /// Group containing `passcode` (linear scan; build a map if called in a loop).
    pub fn group_of_passcode(&self, passcode: u32) -> Option<Gid> {
        (0..self.len()).find(|&g| self.passcodes(g as Gid).contains(&passcode)).map(|g| g as Gid)
    }

    /// Groups of a pool with positive weight.
    pub fn pool_groups(&self, pool: Pool) -> Vec<Gid> {
        (0..self.len() as Gid).filter(|&g| self.weight(pool, g) > 0).collect()
    }

    /// Build a table from raw groups. Groups keep the given order; `n_md` counts the leading
    /// groups with `wm > 0` (callers put Master Duel groups first).
    pub fn from_raw(groups: &[RawGroup]) -> Result<CardTable, DataError> {
        if groups.len() > Gid::MAX as usize {
            return Err(DataError::Invalid("too many groups".into()));
        }
        fn intern<T: Copy + PartialEq + Ord>(dict: &mut Vec<T>, values: impl Iterator<Item = T>) {
            for v in values {
                if !dict.contains(&v) {
                    dict.push(v);
                }
            }
            dict.sort();
        }
        let mut dicts = Dicts::default();
        intern(&mut dicts.frame, groups.iter().map(|g| g.frame));
        intern(&mut dicts.attribute, groups.iter().map(|g| g.attribute));
        intern(&mut dicts.race, groups.iter().map(|g| g.race));
        intern(&mut dicts.level_nm, groups.iter().map(|g| g.level_nm));
        intern(&mut dicts.atk, groups.iter().map(|g| g.atk));
        intern(&mut dicts.def, groups.iter().map(|g| g.def));
        for (name, len) in [
            ("frame", dicts.frame.len()),
            ("attribute", dicts.attribute.len()),
            ("race", dicts.race.len()),
            ("level", dicts.level_nm.len()),
            ("atk", dicts.atk.len()),
            ("def", dicts.def.len()),
        ] {
            if len > 255 {
                return Err(DataError::Invalid(format!("{name} has more than 255 values")));
            }
        }
        if dicts.frame.len() > 15 || dicts.level_nm.len() > 16 {
            return Err(DataError::Invalid("frame/level dictionaries exceed the feedback code".into()));
        }
        let pos = |d: &[u8], v: u8| d.iter().position(|&x| x == v).unwrap() as u8;
        let mut t = CardTable { dicts: dicts.clone(), ..Default::default() };
        t.ids_off.push(0);
        for g in groups {
            if !(1..=15).contains(&g.level_n) || g.level_nm & (1 << g.level_n) == 0 {
                return Err(DataError::Invalid(format!("bad level {} / nm {}", g.level_n, g.level_nm)));
            }
            t.frame.push(pos(&dicts.frame, g.frame));
            t.attribute.push(pos(&dicts.attribute, g.attribute));
            t.race.push(dicts.race.iter().position(|&x| x == g.race).unwrap() as u8);
            t.level.push(dicts.level_nm.iter().position(|&x| x == g.level_nm).unwrap() as u8);
            t.level_n.push(g.level_n);
            t.atk.push(dicts.atk.iter().position(|&x| x == g.atk).unwrap() as u8);
            t.def.push(dicts.def.iter().position(|&x| x == g.def).unwrap() as u8);
            t.wm.push(g.wm);
            t.wa.push(g.wa);
            t.rep_passcode.push(g.rep_passcode);
            t.ids.extend_from_slice(&g.passcodes);
            t.ids_off.push(t.ids.len() as u32);
        }
        t.n_md = groups.iter().take_while(|g| g.wm > 0).count();
        if groups[t.n_md..].iter().any(|g| g.wm > 0) {
            return Err(DataError::Invalid("Master Duel groups must come first".into()));
        }
        // A level match id must determine the rule level, because reveals are built from it.
        for id in 0..dicts.level_nm.len() {
            let ns: Vec<u8> =
                (0..t.len()).filter(|&g| t.level[g] as usize == id).map(|g| t.level_n[g]).collect();
            if ns.windows(2).any(|w| w[0] != w[1]) {
                return Err(DataError::Invalid(format!(
                    "level nm {} has several rule levels",
                    dicts.level_nm[id]
                )));
            }
        }
        t.data_hash = fnv1a(&t.body_bytes());
        Ok(t)
    }

    /// Rule level of a level match id.
    pub fn level_n_of_id(&self, id: u8) -> u8 {
        let g = self.level.iter().position(|&x| x == id).expect("unused level id");
        self.level_n[g]
    }

    fn body_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(self.len() * 20 + 4096);
        let d = &self.dicts;
        put_u16(&mut b, d.frame.len() as u16);
        b.extend_from_slice(&d.frame);
        put_u16(&mut b, d.attribute.len() as u16);
        b.extend_from_slice(&d.attribute);
        put_u16(&mut b, d.race.len() as u16);
        d.race.iter().for_each(|&v| put_u32(&mut b, v));
        put_u16(&mut b, d.level_nm.len() as u16);
        d.level_nm.iter().for_each(|&v| put_u16(&mut b, v));
        put_u16(&mut b, d.atk.len() as u16);
        d.atk.iter().for_each(|&v| put_u16(&mut b, v as u16));
        put_u16(&mut b, d.def.len() as u16);
        d.def.iter().for_each(|&v| put_u16(&mut b, v as u16));
        for col in [
            &self.frame,
            &self.attribute,
            &self.race,
            &self.level,
            &self.level_n,
            &self.atk,
            &self.def,
            &self.wm,
            &self.wa,
        ] {
            b.extend_from_slice(col);
        }
        self.rep_passcode.iter().for_each(|&v| put_u32(&mut b, v));
        self.ids_off.iter().for_each(|&v| put_u32(&mut b, v));
        self.ids.iter().for_each(|&v| put_u32(&mut b, v));
        b
    }

    /// Serialize to the `cards.bin` format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let body = self.body_bytes();
        let mut b = Vec::with_capacity(body.len() + 24);
        b.extend_from_slice(MAGIC);
        put_u16(&mut b, FORMAT_VERSION);
        put_u16(&mut b, 0);
        put_u32(&mut b, self.len() as u32);
        put_u32(&mut b, self.n_md as u32);
        b.extend_from_slice(&fnv1a(&body).to_le_bytes());
        b.extend_from_slice(&body);
        b
    }

    /// Parse a `cards.bin` file.
    pub fn from_bytes(bytes: &[u8]) -> Result<CardTable, DataError> {
        let mut r = Reader { b: bytes, pos: 0 };
        if r.take(4)? != MAGIC {
            return Err(DataError::BadMagic);
        }
        let version = r.u16()?;
        if version != FORMAT_VERSION {
            return Err(DataError::UnsupportedVersion(version));
        }
        r.u16()?;
        let n = r.u32()? as usize;
        let n_md = r.u32()? as usize;
        let hash = r.u64()?;
        let body_start = r.pos;
        if fnv1a(&bytes[body_start..]) != hash {
            return Err(DataError::Invalid("hash mismatch".into()));
        }
        let mut d = Dicts::default();
        let c = r.u16()? as usize;
        d.frame = r.take(c)?.to_vec();
        let c = r.u16()? as usize;
        d.attribute = r.take(c)?.to_vec();
        let c = r.u16()? as usize;
        d.race = (0..c).map(|_| r.u32()).collect::<Result<_, _>>()?;
        let c = r.u16()? as usize;
        d.level_nm = (0..c).map(|_| r.u16()).collect::<Result<_, _>>()?;
        let c = r.u16()? as usize;
        d.atk = (0..c).map(|_| r.u16().map(|v| v as i16)).collect::<Result<_, _>>()?;
        let c = r.u16()? as usize;
        d.def = (0..c).map(|_| r.u16().map(|v| v as i16)).collect::<Result<_, _>>()?;
        let mut t = CardTable { dicts: d, n_md, data_hash: hash, ..Default::default() };
        t.frame = r.take(n)?.to_vec();
        t.attribute = r.take(n)?.to_vec();
        t.race = r.take(n)?.to_vec();
        t.level = r.take(n)?.to_vec();
        t.level_n = r.take(n)?.to_vec();
        t.atk = r.take(n)?.to_vec();
        t.def = r.take(n)?.to_vec();
        t.wm = r.take(n)?.to_vec();
        t.wa = r.take(n)?.to_vec();
        t.rep_passcode = (0..n).map(|_| r.u32()).collect::<Result<_, _>>()?;
        t.ids_off = (0..=n).map(|_| r.u32()).collect::<Result<_, _>>()?;
        let total = *t.ids_off.last().unwrap() as usize;
        t.ids = (0..total).map(|_| r.u32()).collect::<Result<_, _>>()?;
        if r.pos != bytes.len() {
            return Err(DataError::Invalid("trailing bytes".into()));
        }
        t.validate()?;
        Ok(t)
    }

    fn validate(&self) -> Result<(), DataError> {
        let d = &self.dicts;
        let check = |col: &[u8], len: usize, name: &str| {
            if col.iter().any(|&v| v as usize >= len) {
                Err(DataError::Invalid(format!("{name} id out of range")))
            } else {
                Ok(())
            }
        };
        check(&self.frame, d.frame.len(), "frame")?;
        check(&self.attribute, d.attribute.len(), "attribute")?;
        check(&self.race, d.race.len(), "race")?;
        check(&self.level, d.level_nm.len(), "level")?;
        check(&self.atk, d.atk.len(), "atk")?;
        check(&self.def, d.def.len(), "def")?;
        if d.frame.len() > 15 || d.level_nm.len() > 16 {
            return Err(DataError::Invalid("dictionary too large".into()));
        }
        if self.level_n.iter().any(|&n| !(1..=15).contains(&n)) {
            return Err(DataError::Invalid("level out of range".into()));
        }
        if self.n_md > self.len()
            || self.wm[..self.n_md].contains(&0)
            || self.wm[self.n_md..].iter().any(|&w| w > 0)
        {
            return Err(DataError::Invalid("Master Duel prefix mismatch".into()));
        }
        if self.ids_off.windows(2).any(|w| w[0] > w[1]) {
            return Err(DataError::Invalid("passcode offsets not monotone".into()));
        }
        Ok(())
    }
}

/// Which card pool the target is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Pool {
    /// Master Duel cards (weight = number of MD passcodes in the group).
    #[default]
    MasterDuel,
    /// Every official monster (fallback when the MD list is suspected incomplete).
    All,
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DataError> {
        let end = self.pos.checked_add(n).ok_or(DataError::Truncated)?;
        let s = self.b.get(self.pos..end).ok_or(DataError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, DataError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, DataError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, DataError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tiny_groups() -> Vec<RawGroup> {
        let g = |frame, attribute, race, level_n: u8, extra_nm: u16, atk, def, wm, pc| RawGroup {
            frame,
            attribute,
            race,
            level_n,
            level_nm: (1 << level_n) | extra_nm,
            atk,
            def,
            wm,
            wa: wm.max(1),
            passcodes: vec![pc],
            rep_passcode: pc,
        };
        vec![
            g(132, 32, 8192, 12, 0, 4000, 4000, 1, 13331639), // Zarc: Fusion/Pendulum
            g(4, 32, 2097152, 10, 0, STAT_UNKNOWN, 0, 1, 83257450), // Fusion
            g(136, 8, 8192, 7, 0, 2500, 2000, 1, 90036274),   // Synchro/Pendulum
            g(8, 32, 8192, 12, 1, 0, 0, 1, 1686814),          // Tzolkin: rule 12, printed 0
            g(16, 16, 1, 1, 1, 0, 0, 1, 65305468),            // FNo.0: rule rank 1, printed 0
            g(2, 32, 8, 12, 0, 5000, 5000, 1, 71111111),      // a level-12 effect monster
            g(32, 16, 1048576, 4, 0, 2300, STAT_NONE, 1, 1861629), // Link-4
            g(2, 1, 1, 4, 0, 1500, 1000, 0, 99999999),        // non-MD
        ]
    }

    #[test]
    fn roundtrip() {
        let t = CardTable::from_raw(&tiny_groups()).unwrap();
        assert_eq!(t.n_md, 7);
        let bytes = t.to_bytes();
        let u = CardTable::from_bytes(&bytes).unwrap();
        assert_eq!(u.to_bytes(), bytes);
        assert_eq!(u.data_hash, t.data_hash);
        assert_eq!(u.group_of_passcode(1686814), Some(3));
        assert_eq!(u.shown(Field::Level, 3), 12);
    }

    #[test]
    fn rejects_corruption() {
        let t = CardTable::from_raw(&tiny_groups()).unwrap();
        let mut bytes = t.to_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(matches!(CardTable::from_bytes(&bytes), Err(DataError::Invalid(_))));
        assert!(matches!(CardTable::from_bytes(&bytes[..10]), Err(DataError::Truncated)));
    }
}
