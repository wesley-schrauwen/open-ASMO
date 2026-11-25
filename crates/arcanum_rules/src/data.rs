//! Rules tables read once from the game data.
//!
//! - Backgrounds: `Rules/backgrnd.mes`, blocks of 10 (description id in `mes/gameback.mes`,
//!   effect id in `rules/effect.mes`, legal race-gender codes, money, items). Blank or `ANY`
//!   allows everyone; `NPC` marks the pre-made characters' own backgrounds, never offered at
//!   creation.
//! - Effects: `rules/effect.mes`, e.g. `st +1, cn +1, resistpoison +10`; race effects are
//!   64 + race. Each line is read as a list of [`Change`]s (0x4e9c20) and applied to a value by
//!   [`RulesData::apply_effects`] (0x4ea6c0).
//! - Portraits: `portrait/gameport.mes` entries named after the race-gender code (`DWM1`, ...).

use arcanum_formats::mes::MesFile;
use arcanum_vfs::Vfs;

use crate::sheet::{FEMALE, PRIMARY_STATS, RACE_COUNT};
use crate::tables::{SKILLS, Schematic, SkillTable, SpellTable, TechTable, parse_schematics};

/// Race-gender codes as used in `gameport.mes` and `backgrnd.mes`.
const RACE_CODES: [&str; RACE_COUNT] = ["HU", "DW", "EL", "HE", "GN", "HA", "HO", "HG"];
const STAT_KEYS: [&str; PRIMARY_STATS] = ["st", "dx", "cn", "be", "in", "pe", "wp", "ch"];
/// First race effect in `rules/effect.mes`.
pub const RACE_EFFECT_BASE: u32 = 64;
/// Resistance kinds, in the character editor's order.
pub const RESISTANCES: usize = 5;
pub const RESIST_KEYS: [&str; RESISTANCES] = [
    "resistdamage",
    "resistmagic",
    "resistfire",
    "resistpoison",
    "resistelectrical",
];
/// Skill keys in `rules/effect.mes`, in `mes/skill.mes` order.
const SKILL_KEYS: [&str; SKILLS] = [
    "bow",
    "dodge",
    "melee",
    "throwing",
    "backstab",
    "pickpocket",
    "prowling",
    "spottrap",
    "gambling",
    "haggle",
    "heal",
    "persuasion",
    "repair",
    "firearms",
    "picklock",
    "armtrap",
];

/// One background block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Background {
    /// Block number in `Rules/backgrnd.mes` (the block starts at `id * 10`).
    pub id: u32,
    /// `mes/gameback.mes` id: title line, then the story.
    pub description: u32,
    /// `rules/effect.mes` id.
    pub effect: Option<u32>,
    /// Legal race-gender codes; empty = everyone.
    pub codes: Vec<String>,
    /// A pre-made character's own background (`NPC`): kept, never offered at creation.
    pub npc: bool,
    /// Gold the background gives (line 4).
    pub money: i32,
    /// Item prototype numbers the background gives (line 5; bad tokens skipped).
    pub items: Vec<u32>,
}

impl Background {
    pub fn allows(&self, race: u8, gender: u8) -> bool {
        !self.npc
            && (self.codes.is_empty()
                || self
                    .codes
                    .iter()
                    .any(|c| c == "ANY" || *c == race_gender_code(race, gender)))
    }
}

/// `DWM`, `HUF`, ...
pub fn race_gender_code(race: u8, gender: u8) -> String {
    format!(
        "{}{}",
        RACE_CODES[race as usize % RACE_COUNT],
        if gender == FEMALE { 'F' } else { 'M' }
    )
}

/// What an effect line does to the primary stats and resistances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Effect {
    pub stats: [i32; PRIMARY_STATS],
    /// Damage, Magick, Fire, Poison, Electrical (`mes/charedit.mes` 12-16 order).
    pub resist: [i32; RESISTANCES],
    /// Skill level changes (levels, not points), `mes/skill.mes` order.
    pub skills: [i32; SKILLS],
    /// `magicpts` and `techpts`: counted as spells known and tech degrees in the aptitude.
    pub magick_points: i32,
    pub tech_points: i32,
    /// `aptitude` (straight onto the magick/tech aptitude) and `alignment` (onto the
    /// alignment; `alignment max` lines are not read).
    pub aptitude: i32,
    pub alignment: i32,
}

/// Everything the character rules need from the game data.
#[derive(Debug, Clone, Default)]
pub struct RulesData {
    pub backgrounds: Vec<Background>,
    /// `(portrait id, file stem)` in id order.
    pub portraits: Vec<(u32, String)>,
    /// Stat and resistance changes per effect id (the flat `+N` / `-N` ones only).
    pub effects: Vec<(u32, Effect)>,
    /// Every change of every effect, by effect id.
    pub changes: std::collections::BTreeMap<u32, Vec<Change>>,
    pub skills: SkillTable,
    pub tech: TechTable,
    pub spells: SpellTable,
    pub schematics: Vec<Schematic>,
    /// The pre-made characters, built from their PC prototypes.
    pub premades: Vec<crate::blueprint::Premade>,
    /// Experience to reach each level: entry L is `Rules/xp_level.mes` line L + 1, the
    /// experience needed to leave level L (entry 0 is 0; loader 0x4a6600, table 0x5f5c94).
    pub xp_levels: Vec<i32>,
}

pub(crate) fn read_mes(vfs: &Vfs, path: &str) -> Result<MesFile, String> {
    let data = vfs.read(path).map_err(|e| format!("{path}: {e}"))?;
    MesFile::parse(&data).map_err(|e| format!("{path}: {e}"))
}

impl RulesData {
    /// Reads `Rules/backgrnd.mes`, `rules/effect.mes`, `portrait/gameport.mes` and the skill,
    /// tech, spell and schematic tables.
    pub fn load(vfs: &Vfs) -> Result<Self, String> {
        let mut data = Self::from_files(
            &read_mes(vfs, "Rules/backgrnd.mes")?,
            &read_mes(vfs, "rules/effect.mes")?,
            &read_mes(vfs, "portrait/gameport.mes")?,
        );
        // No screen needs these yet: a missing or broken table is left empty rather than losing
        // character creation.
        let optional = |path: &str| read_mes(vfs, path).ok();
        if let Some(m) = optional("mes/skill.mes") {
            data.skills = SkillTable::from_mes(&m);
        }
        if let Some(m) = optional("mes/tech.mes") {
            data.tech = TechTable::from_mes(&m);
        }
        if let Some(m) = optional("mes/spell.mes") {
            data.spells = SpellTable::from_mes(&m);
            if let Some(list) = optional("rules/spelllist.mes") {
                data.spells.read_costs(&list);
            }
        }
        if let Some(m) = optional("rules/schematic.mes") {
            data.schematics = parse_schematics(&m);
        }
        if let Some(m) = optional("Rules/xp_level.mes") {
            data.xp_levels = parse_xp_levels(&m);
        }
        data.load_premades(vfs);
        Ok(data)
    }

    pub fn from_files(backgrounds: &MesFile, effects: &MesFile, portraits: &MesFile) -> Self {
        let mut list = Vec::new();
        for id in 0..1000u32 {
            let Some(description) = backgrounds
                .get(id * 10)
                .and_then(|s| s.trim().parse::<u32>().ok())
            else {
                continue;
            };
            let effect = backgrounds
                .get(id * 10 + 1)
                .and_then(|s| s.trim().parse().ok());
            let codes: Vec<String> = backgrounds
                .get(id * 10 + 2)
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_ascii_uppercase)
                .collect();
            let npc = codes.iter().any(|c| c == "NPC");
            let money = backgrounds
                .get(id * 10 + 3)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let items = backgrounds
                .get(id * 10 + 4)
                .unwrap_or("")
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            list.push(Background {
                id,
                description,
                effect,
                codes,
                npc,
                money,
                items,
            });
        }
        let mut stat_effects = Vec::new();
        let mut changes = std::collections::BTreeMap::new();
        for id in 0..2000u32 {
            if let Some(text) = effects.get(id) {
                let effect = parse_effect(text);
                if effect != Effect::default() {
                    stat_effects.push((id, effect));
                }
                let list = parse_changes(text);
                if !list.is_empty() {
                    changes.insert(id, list);
                }
            }
        }
        let mut portrait_list = Vec::new();
        for id in 1000..2000u32 {
            if let Some(stem) = portraits.get(id) {
                portrait_list.push((id, stem.trim().to_string()));
            }
        }
        Self {
            backgrounds: list,
            portraits: portrait_list,
            effects: stat_effects,
            changes,
            ..Self::default()
        }
    }

    pub(crate) fn effect(&self, id: u32) -> Effect {
        self.effects
            .iter()
            .find(|(e, _)| *e == id)
            .map(|(_, m)| *m)
            .unwrap_or_default()
    }

    /// `value` after the effects `ids` (0x4ea6c0): over every change aimed at `target`, the
    /// percentages are summed, the multipliers and divisors multiplied, the additions summed;
    /// the highest `min` is a floor and the lowest `max` a cap. Applied in that order: percent,
    /// multiply, divide, add, floor, cap.
    pub fn apply_effects(&self, target: EffectTarget, value: i32, ids: &[u32]) -> i32 {
        let (mut add, mut multiply, mut divide, mut percent) = (0, 1, 1, 0);
        let (mut floor, mut cap) = (-1, -1);
        for change in ids
            .iter()
            .filter_map(|id| self.changes.get(id))
            .flatten()
            .filter(|c| c.target == target)
        {
            match change.kind {
                ChangeKind::Add => add += change.value,
                ChangeKind::Multiply => multiply *= change.value,
                ChangeKind::Divide => divide *= change.value,
                ChangeKind::AtLeast => floor = floor.max(change.value),
                ChangeKind::AtMost => {
                    cap = if cap == -1 {
                        change.value
                    } else {
                        cap.min(change.value)
                    }
                }
                ChangeKind::Percent => percent += change.value,
            }
        }
        let mut v = value;
        if percent != 0 {
            v = (percent + 100) * v / 100;
        }
        if multiply != 1 {
            v *= multiply;
        }
        if divide != 1 && divide != 0 {
            v /= divide;
        }
        v += add;
        if floor != -1 && v < floor {
            v = floor;
        }
        if cap != -1 && cap < v {
            v = cap;
        }
        v
    }
}

/// The experience table from `Rules/xp_level.mes`: 51 entries, entry i line i + 1, entry 0
/// always 0.
pub fn parse_xp_levels(mes: &MesFile) -> Vec<i32> {
    let mut out: Vec<i32> = (1..=crate::LEVEL_LIMIT as u32)
        .map(|id| mes.get(id).and_then(|t| t.trim().parse().ok()).unwrap_or(0))
        .collect();
    out[0] = 0;
    out
}

/// What a change in an effect line acts on: the five key tables of 0x4e9c20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTarget {
    /// A stat by `mes/stat.mes` id ([`STAT_NAMES`], table 0x5b9ba8).
    Stat(u8),
    /// A skill in `mes/skill.mes` order (tables 0x5b9c18 and 0x5b9c48).
    Skill(u8),
    /// A resistance in [`RESIST_KEYS`] order (table 0x5b9c58).
    Resist(u8),
    /// A tech discipline's expertise (table 0x5b9c6c).
    Expertise(u8),
    /// [`OTHER_KEYS`]: max hit points, max fatigue, reaction, criticals... (table 0x5b9c8c).
    Other(u8),
}

/// How a change acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// `+N` / `-N`.
    Add,
    /// `*N`.
    Multiply,
    /// `/N`.
    Divide,
    /// `min N`: the value is at least N.
    AtLeast,
    /// `max N`: the value is at most N.
    AtMost,
    /// `+N%` / `-N%`.
    Percent,
}

/// One change of an effect line, e.g. `speed -50%`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub target: EffectTarget,
    pub kind: ChangeKind,
    pub value: i32,
}

/// The stat keys, by stat id (table 0x5b9ba8).
pub const STAT_NAMES: [&str; 28] = [
    "st",
    "dx",
    "cn",
    "be",
    "in",
    "pe",
    "wp",
    "ch",
    "carry",
    "damage",
    "ac",
    "speed",
    "healrate",
    "poisonrate",
    "beautyreaction",
    "maxfollowers",
    "aptitude",
    "level",
    "xpsUNUSED",
    "alignment",
    "fateUNUSED",
    "unspentUNUSED",
    "magicpts",
    "techpts",
    "poisonlevel",
    "age",
    "gender",
    "race",
];
/// The expertise keys (table 0x5b9c6c).
const EXPERTISE_KEYS: [&str; 8] = [
    "expertiseanatomical",
    "expertisechemistry",
    "expertiseelectric",
    "expertiseexplosives",
    "expertisegun_smithy",
    "expertisemechanical",
    "expertisesmithy",
    "expertisetherapeutics",
];
/// The other keys (table 0x5b9c8c).
pub const OTHER_KEYS: [&str; 10] = [
    "maxhps",
    "maxfatigue",
    "reaction",
    "crithitchance",
    "crithiteffect",
    "critfailchance",
    "critfaileffect",
    "badreactionadj",
    "goodreactionadj",
    "xpgain",
];
/// `maxhps` and `maxfatigue` in [`OTHER_KEYS`].
pub const MAX_HIT_POINTS: EffectTarget = EffectTarget::Other(0);
pub const MAX_FATIGUE: EffectTarget = EffectTarget::Other(1);

fn effect_target(key: &str) -> Option<EffectTarget> {
    let find = |keys: &[&str]| {
        keys.iter()
            .position(|k| k.eq_ignore_ascii_case(key))
            .map(|i| i as u8)
    };
    find(&STAT_NAMES)
        .map(EffectTarget::Stat)
        .or_else(|| find(&SKILL_KEYS).map(EffectTarget::Skill))
        .or_else(|| find(&RESIST_KEYS).map(EffectTarget::Resist))
        .or_else(|| find(&EXPERTISE_KEYS).map(EffectTarget::Expertise))
        .or_else(|| find(&OTHER_KEYS).map(EffectTarget::Other))
}

/// The changes in an effect line (0x4e9c20): tokens split at spaces and commas, each a key then
/// `+N` / `-N` (`%` at the end: a percentage), `*N`, `/N`, or `min N` / `max N`, compared without
/// case (0x58adb0). An unknown key
/// ends the line ("Effect: ERROR: Invalid symbol"); an unknown value form is skipped.
pub fn parse_changes(text: &str) -> Vec<Change> {
    let atoi = |t: &str| {
        let t = t.trim_start_matches('+');
        let end = t
            .char_indices()
            .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && c == '-')))
            .map_or(t.len(), |(i, _)| i);
        t[..end].parse::<i32>().unwrap_or(0)
    };
    let text = text.split("//").next().unwrap_or("");
    let mut tokens = text.split([' ', ',', '\t']).filter(|t| !t.is_empty());
    let mut out = Vec::new();
    while let Some(key) = tokens.next() {
        let Some(target) = effect_target(key) else {
            break;
        };
        let Some(value) = tokens.next() else { break };
        let (kind, v) = match value.to_ascii_lowercase().as_str() {
            "min" => (ChangeKind::AtLeast, tokens.next().map_or(0, atoi)),
            "max" => (ChangeKind::AtMost, tokens.next().map_or(0, atoi)),
            _ if value.starts_with(['+', '-']) => (
                if value.ends_with('%') {
                    ChangeKind::Percent
                } else {
                    ChangeKind::Add
                },
                atoi(value),
            ),
            _ if value.starts_with('*') => (ChangeKind::Multiply, atoi(&value[1..])),
            _ if value.starts_with('/') => (ChangeKind::Divide, atoi(&value[1..])),
            _ => continue,
        };
        out.push(Change {
            target,
            kind,
            value: v,
        });
    }
    out
}

/// The primary stat changes in an effect line such as `st +1, cn +1, ch -1, repair +2`.
pub fn parse_stat_effect(text: &str) -> [i32; PRIMARY_STATS] {
    parse_effect(text).stats
}

/// The stat, resistance and skill changes in an effect line.
pub fn parse_effect(text: &str) -> Effect {
    let mut out = Effect::default();
    for part in text.split(',') {
        let mut words = part.split_whitespace();
        let (Some(key), Some(value)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(v) = value.trim_start_matches('+').parse::<i32>() else {
            continue;
        };
        if let Some(i) = STAT_KEYS.iter().position(|k| key.eq_ignore_ascii_case(k)) {
            out.stats[i] += v;
        } else if let Some(i) = RESIST_KEYS.iter().position(|k| key.eq_ignore_ascii_case(k)) {
            out.resist[i] += v;
        } else if let Some(i) = SKILL_KEYS.iter().position(|k| key.eq_ignore_ascii_case(k)) {
            out.skills[i] += v;
        } else {
            match key.to_ascii_lowercase().as_str() {
                "magicpts" => out.magick_points += v,
                "techpts" => out.tech_points += v,
                "aptitude" => out.aptitude += v,
                "alignment" => out.alignment += v,
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tables_do_not_stop_character_creation() {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("Rules/backgrnd.mes", "{0}{1000}{1}{86}{2}{}"),
            ("rules/effect.mes", "{65}{st +1}"),
            ("portrait/gameport.mes", "{1005}{HUM1}"),
        ] {
            let file = dir.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_dir(dir.path());
        let data =
            RulesData::load(&vfs).expect("skill/tech/spell/schematic tables are optional for now");
        assert_eq!((data.backgrounds.len(), data.portraits.len()), (1, 1));
        assert!(
            data.skills.skills.is_empty() && data.schematics.is_empty() && data.premades.is_empty()
        );
    }

    #[test]
    fn effect_lines_read_every_form_of_change() {
        use ChangeKind::*;
        use EffectTarget::*;
        let c = |target, kind, value| Change {
            target,
            kind,
            value,
        };
        assert_eq!(
            parse_changes(
                "speed +10, damage max 0, resistdamage min 95, ST -6, Melee +1, maxfatigue +5%, wp *2, cn /3"
            ),
            [
                c(Stat(11), Add, 10),
                c(Stat(9), AtMost, 0),
                c(Resist(0), AtLeast, 95),
                c(Stat(0), Add, -6),
                c(Skill(2), Add, 1),
                c(Other(1), Percent, 5),
                c(Stat(6), Multiply, 2),
                c(Stat(2), Divide, 3)
            ]
        );
        assert_eq!(
            parse_changes("BE -4, CN +2 DX +1"),
            [c(Stat(3), Add, -4), c(Stat(2), Add, 2), c(Stat(1), Add, 1)],
            "a missing comma"
        );
        assert_eq!(
            parse_changes("st +1, nonsense +2, dx +1"),
            [c(Stat(0), Add, 1)],
            "an unknown key ends the line"
        );
    }

    #[test]
    fn effects_apply_percent_then_multiply_divide_add_floor_cap() {
        let r = RulesData::from_files(&MesFile::default(), &MesFile::parse(b"{1}{speed -50%}{2}{speed +4}{3}{speed max 9}{4}{speed min 30}{5}{speed *3, speed /2}").unwrap(), &MesFile::default());
        let speed = |v, ids: &[u32]| r.apply_effects(EffectTarget::Stat(11), v, ids);
        assert_eq!(speed(10, &[]), 10);
        assert_eq!(speed(10, &[1, 2]), 9, "half of 10, then + 4");
        assert_eq!(speed(10, &[1, 1]), 0, "-100%");
        assert_eq!(speed(20, &[2, 3]), 9, "the cap comes last");
        assert_eq!(speed(10, &[4, 3]), 9, "floor 30, then cap 9");
        assert_eq!(speed(11, &[5]), 16, "x3 then /2");
        assert_eq!(
            r.apply_effects(EffectTarget::Stat(0), 10, &[1, 2]),
            10,
            "another stat"
        );
    }

    #[test]
    fn effects_carry_resistances() {
        let e = parse_effect("st +1, cn +1, melee +2, resistpoison +10, be -2");
        assert_eq!(e.stats, [1, 0, 1, -2, 0, 0, 0, 0]);
        assert_eq!(e.resist, [0, 0, 0, 10, 0]);
    }
}
