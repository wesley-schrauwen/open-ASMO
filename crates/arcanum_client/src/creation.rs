//! Character creation rules, read from the game's data, without any UI.
//!
//! Matches the original (Arcanum.exe character creation, window index 12; see NOTES.md):
//! - Races 0-7 (`MainMenu.mes` 720+); only Human, Elf, Half-Elf and Half-Orc may be female (race
//!   table at 0x5c5170). Gender 0 female, 1 male (`MainMenu.mes` 740+).
//! - Portraits: the `portrait/gameport.mes` entries named after the race-gender code (`DWM1`,
//!   `DWM2`, ...); changing race or gender goes back to the first one.
//! - Backgrounds: `Rules/backgrnd.mes`, blocks of 10 (description id in `mes/gameback.mes`,
//!   effect id in `rules/effect.mes`, legal race-gender codes, money, items). Blank or `ANY`
//!   allows everyone; `NPC` marks the pre-made characters' own backgrounds, never offered here.
//! - Starting stats: 8 in each primary stat, plus the race effect (`rules/effect.mes` 64 + race)
//!   and the background effect.

use arcanum_formats::mes::MesFile;
use arcanum_vfs::Vfs;

pub const RACE_COUNT: usize = 8;
/// Race-gender codes as used in `gameport.mes` and `backgrnd.mes`.
const RACE_CODES: [&str; RACE_COUNT] = ["HU", "DW", "EL", "HE", "GN", "HA", "HO", "HG"];
/// Which races allow a female character (Arcanum.exe race table 0x5c5170).
pub const FEMALE_ALLOWED: [bool; RACE_COUNT] = [true, false, true, true, false, false, true, false];
pub const FEMALE: u8 = 0;
pub const MALE: u8 = 1;
/// The eight primary stats in `mes/stat.mes` order: ST DX CN BE IN PE WP CH.
pub const PRIMARY_STATS: usize = 8;
const STAT_KEYS: [&str; PRIMARY_STATS] = ["st", "dx", "cn", "be", "in", "pe", "wp", "ch"];
pub const BASE_STAT: i32 = 8;
/// First race effect in `rules/effect.mes`.
const RACE_EFFECT_BASE: u32 = 64;

/// A background the player may pick.
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
}

impl Background {
    pub fn allows(&self, race: u8, gender: u8) -> bool {
        self.codes.is_empty() || self.codes.iter().any(|c| c == "ANY" || *c == race_gender_code(race, gender))
    }
}

/// `DWM`, `HUF`, ...
pub fn race_gender_code(race: u8, gender: u8) -> String {
    format!("{}{}", RACE_CODES[race as usize % RACE_COUNT], if gender == FEMALE { 'F' } else { 'M' })
}

/// Everything character creation needs from the game data.
#[derive(Debug, Clone, Default)]
pub struct CreationRules {
    pub backgrounds: Vec<Background>,
    /// `(portrait id, file stem)` in id order.
    pub portraits: Vec<(u32, String)>,
    /// Stat and resistance changes per effect id.
    pub effects: Vec<(u32, Effect)>,
}

/// What an effect line in `rules/effect.mes` does to the stats this screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Effect {
    pub stats: [i32; PRIMARY_STATS],
    /// Damage, Magick, Fire, Poison, Electrical (`mes/charedit.mes` 12-16 order).
    pub resist: [i32; RESISTANCES],
}

/// Resistance kinds, in the character editor's order.
pub const RESISTANCES: usize = 5;
const RESIST_KEYS: [&str; RESISTANCES] = ["resistdamage", "resistmagic", "resistfire", "resistpoison", "resistelectrical"];

impl CreationRules {
    /// Reads `Rules/backgrnd.mes`, `rules/effect.mes` and `portrait/gameport.mes`.
    pub fn load(vfs: &Vfs) -> Result<Self, String> {
        let mes = |path: &str| -> Result<MesFile, String> {
            let data = vfs.read(path).map_err(|e| format!("{path}: {e}"))?;
            MesFile::parse(&data).map_err(|e| format!("{path}: {e}"))
        };
        Ok(Self::from_files(&mes("Rules/backgrnd.mes")?, &mes("rules/effect.mes")?, &mes("portrait/gameport.mes")?))
    }

    pub fn from_files(backgrounds: &MesFile, effects: &MesFile, portraits: &MesFile) -> Self {
        let mut list = Vec::new();
        for id in 0..1000u32 {
            let Some(description) = backgrounds.get(id * 10).and_then(|s| s.trim().parse::<u32>().ok()) else { continue };
            let effect = backgrounds.get(id * 10 + 1).and_then(|s| s.trim().parse().ok());
            let codes: Vec<String> = backgrounds.get(id * 10 + 2).unwrap_or("").split_whitespace().map(str::to_ascii_uppercase).collect();
            if codes.iter().any(|c| c == "NPC") {
                continue;
            }
            list.push(Background { id, description, effect, codes });
        }
        let mut stat_effects = Vec::new();
        for id in 0..2000u32 {
            if let Some(text) = effects.get(id) {
                let effect = parse_effect(text);
                if effect != Effect::default() {
                    stat_effects.push((id, effect));
                }
            }
        }
        let mut portrait_list = Vec::new();
        for id in 1000..2000u32 {
            if let Some(stem) = portraits.get(id) {
                portrait_list.push((id, stem.trim().to_string()));
            }
        }
        Self { backgrounds: list, portraits: portrait_list, effects: stat_effects }
    }

    /// Backgrounds legal for this race and gender, in file order (the first is "no background").
    pub fn backgrounds_for(&self, race: u8, gender: u8) -> Vec<&Background> {
        self.backgrounds.iter().filter(|b| b.allows(race, gender)).collect()
    }

    /// Portrait file stems for this race and gender (`DWM1`, `DWM2`, ...).
    pub fn portraits_for(&self, race: u8, gender: u8) -> Vec<&str> {
        let code = race_gender_code(race, gender);
        self.portraits
            .iter()
            .filter(|(_, stem)| stem.len() > code.len() && stem.starts_with(&code) && stem[code.len()..].chars().all(|c| c.is_ascii_digit()))
            .map(|(_, stem)| stem.as_str())
            .collect()
    }

    fn effect(&self, id: u32) -> Effect {
        self.effects.iter().find(|(e, _)| *e == id).map(|(_, m)| *m).unwrap_or_default()
    }

    /// The race and background effects together.
    fn character_effects(&self, character: &NewCharacter) -> Effect {
        let race = self.effect(RACE_EFFECT_BASE + u32::from(character.race));
        let background = self.background(character).and_then(|b| b.effect).map(|e| self.effect(e)).unwrap_or_default();
        Effect {
            stats: std::array::from_fn(|i| race.stats[i] + background.stats[i]),
            resist: std::array::from_fn(|i| race.resist[i] + background.resist[i]),
        }
    }

    /// Starting primary stats before any points are spent: base 8, plus race and background.
    pub fn starting_stats(&self, character: &NewCharacter) -> [i32; PRIMARY_STATS] {
        let e = self.character_effects(character);
        std::array::from_fn(|i| BASE_STAT + e.stats[i])
    }

    /// Current primary stats: the starting stats plus points spent.
    pub fn stats(&self, character: &NewCharacter) -> [i32; PRIMARY_STATS] {
        let start = self.starting_stats(character);
        std::array::from_fn(|i| start[i] + character.bought[i])
    }

    /// Resistances from race and background, in percent.
    pub fn resistances(&self, character: &NewCharacter) -> [i32; RESISTANCES] {
        self.character_effects(character).resist
    }

    /// Raises primary stat `i` by one, spending a point (Arcanum.exe 0x55a80a).
    pub fn raise_stat(&self, character: &mut NewCharacter, i: usize) -> Result<(), EditError> {
        if self.stats(character)[i] >= race_max(character.race, i) {
            return Err(EditError::AtMaximum);
        }
        if character.unspent() < STAT_COST {
            return Err(EditError::NoPoints);
        }
        character.bought[i] += 1;
        Ok(())
    }

    /// Lowers primary stat `i` by one, refunding a point; not below its starting value (0x55a997).
    pub fn lower_stat(&self, character: &mut NewCharacter, i: usize) -> Result<(), EditError> {
        if character.bought[i] == 0 {
            return Err(if self.stats(character)[i] <= MIN_STAT { EditError::AtMinimum } else { EditError::BelowAccepted });
        }
        character.bought[i] -= 1;
        Ok(())
    }

    /// The derived stats for a level-1 character (Arcanum.exe 0x4b0740, 0x43d5a0, 0x45d670).
    pub fn derived(&self, character: &NewCharacter) -> Derived {
        let s = self.stats(character);
        let [st, dx, cn, be, _in, _pe, wp, ch] = s;
        let level = 1;
        let damage_bonus = {
            let d = st - 10;
            let d = if d < 0 { d / 2 } else { d };
            if st >= 20 { d * 2 } else { d }
        };
        Derived {
            carry_weight: 500 * st,
            damage_bonus,
            ac_adjustment: dx - 10,
            speed: dx + if dx >= 20 { 5 } else { 0 },
            heal_rate: (cn + 1) / 3,
            poison_recovery: cn,
            reaction: reaction_modifier(be),
            max_followers: ch / 4,
            hit_points: 4 * character.hp_bought + wp + 2 * (level + st) + 4,
            fatigue: 4 * character.fatigue_bought + wp + 2 * (level + cn) + 4,
        }
    }

    pub fn background(&self, character: &NewCharacter) -> Option<&Background> {
        self.backgrounds_for(character.race, character.gender).get(character.background).copied()
    }

    pub fn portrait(&self, character: &NewCharacter) -> Option<&str> {
        self.portraits_for(character.race, character.gender).get(character.portrait).copied()
    }
}

/// The primary stat changes in an effect line such as `st +1, cn +1, ch -1, repair +2`.
pub fn parse_stat_effect(text: &str) -> [i32; PRIMARY_STATS] {
    parse_effect(text).stats
}

/// The stat and resistance changes in an effect line.
pub fn parse_effect(text: &str) -> Effect {
    let mut out = Effect::default();
    for part in text.split(',') {
        let mut words = part.split_whitespace();
        let (Some(key), Some(value)) = (words.next(), words.next()) else { continue };
        let Ok(v) = value.trim_start_matches('+').parse::<i32>() else { continue };
        if let Some(i) = STAT_KEYS.iter().position(|k| key.eq_ignore_ascii_case(k)) {
            out.stats[i] += v;
        } else if let Some(i) = RESIST_KEYS.iter().position(|k| key.eq_ignore_ascii_case(k)) {
            out.resist[i] += v;
        }
    }
    out
}

/// Points a new level-1 character starts with (stat 21 in the prototypes).
pub const STARTING_POINTS: i32 = 5;
/// Character points per step of a primary stat, hit points or fatigue (table 0x5b52e0: flat 1).
pub const STAT_COST: i32 = 1;
/// No stat goes below this (table 0x5b5194).
pub const MIN_STAT: i32 = 1;

/// Why a +/- click was refused, with its `mes/charedit.mes` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// 17 "Stat has reached its maximum value."
    AtMaximum,
    /// 18 "You haven't enough character points."
    NoPoints,
    /// 19 "You cannot reduce this stat below its last accepted level."
    BelowAccepted,
    /// 20 "Stat has reached its minimum value."
    AtMinimum,
}

impl EditError {
    pub fn message_id(self) -> u32 {
        match self {
            Self::AtMaximum => 17,
            Self::NoPoints => 18,
            Self::BelowAccepted => 19,
            Self::AtMinimum => 20,
        }
    }
}

/// The highest value primary stat `stat` can reach for `race` (Arcanum.exe 0x4b0fd0; 20 unless
/// the race raises it).
pub fn race_max(race: u8, stat: usize) -> i32 {
    // Indices: 0 ST, 1 DX, 2 CN, 3 BE, 4 IN, 5 PE, 6 WP, 7 CH.
    match (race, stat) {
        (1, 0) | (1, 2) => 21,                 // dwarf: ST, CN
        (2, 1) | (2, 3) | (2, 6) => 21,        // elf: DX, BE, WP
        (3, 1) => 21,                          // half-elf: DX
        (4, 6) => 22,                          // gnome: WP
        (5, 1) => 22,                          // halfling: DX
        (6, 0) | (6, 2) => 21,                 // half-orc: ST, CN
        (7, 0) => 24,                          // half-ogre: ST
        _ => 20,
    }
}

/// Reaction modifier for a Beauty value (table 0x5b5330).
pub fn reaction_modifier(beauty: i32) -> i32 {
    const UP: [i32; 10] = [0, 3, 7, 12, 18, 25, 33, 42, 52, 65];
    match beauty {
        b if b >= 20 => 10 * b - 100,
        b if b >= 10 => UP[(b - 10) as usize],
        b if b >= 1 => -UP[(10 - b) as usize],
        _ => -UP[9],
    }
}

/// Experience needed for level 2 (`Rules/xp_level.mes` 2).
pub const XP_FOR_LEVEL_2: i32 = 2100;

/// The derived stats the character editor shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Derived {
    pub carry_weight: i32,
    pub damage_bonus: i32,
    pub ac_adjustment: i32,
    pub speed: i32,
    pub heal_rate: i32,
    pub poison_recovery: i32,
    pub reaction: i32,
    pub max_followers: i32,
    pub hit_points: i32,
    pub fatigue: i32,
}

impl Derived {
    /// Stats 8-15 in `mes/stat.mes` order.
    pub fn by_stat_id(&self) -> [i32; 8] {
        [
            self.carry_weight,
            self.damage_bonus,
            self.ac_adjustment,
            self.speed,
            self.heal_rate,
            self.poison_recovery,
            self.reaction,
            self.max_followers,
        ]
    }
}

/// The character being created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCharacter {
    pub name: String,
    pub race: u8,
    pub gender: u8,
    /// Index into [`CreationRules::backgrounds_for`].
    pub background: usize,
    /// Index into [`CreationRules::portraits_for`].
    pub portrait: usize,
    /// Points spent on each primary stat in the character editor.
    pub bought: [i32; PRIMARY_STATS],
    /// Points spent on hit points and fatigue (+4 each per point).
    pub hp_bought: i32,
    pub fatigue_bought: i32,
}

impl Default for NewCharacter {
    /// As the "create your own" template (prototype 016086): a human male, first portrait, no
    /// background, no name.
    fn default() -> Self {
        Self { name: String::new(), race: 0, gender: MALE, background: 0, portrait: 0, bought: [0; PRIMARY_STATS], hp_bought: 0, fatigue_bought: 0 }
    }
}

/// Longest name: the original's buffer holds 23 bytes including the terminator.
pub const MAX_NAME: usize = 22;

impl NewCharacter {
    /// Steps the race by `delta`, skipping races that do not allow the current gender. Resets
    /// portrait and background.
    pub fn step_race(&mut self, delta: i32) {
        let mut race = self.race as i32;
        for _ in 0..RACE_COUNT {
            race = (race + delta).rem_euclid(RACE_COUNT as i32);
            if self.gender == MALE || FEMALE_ALLOWED[race as usize] {
                break;
            }
        }
        self.race = race as u8;
        self.portrait = 0;
        self.background = 0;
    }

    /// Switches gender if the race allows it. Returns whether it changed.
    pub fn toggle_gender(&mut self) -> bool {
        let target = if self.gender == MALE { FEMALE } else { MALE };
        if target == FEMALE && !FEMALE_ALLOWED[self.race as usize] {
            return false;
        }
        self.gender = target;
        self.portrait = 0;
        self.background = 0;
        true
    }

    /// Steps through `count` choices with wrap.
    fn step(index: &mut usize, count: usize, delta: i32) {
        if count > 0 {
            *index = (*index as i64 + i64::from(delta)).rem_euclid(count as i64) as usize;
        }
    }

    pub fn step_portrait(&mut self, rules: &CreationRules, delta: i32) {
        Self::step(&mut self.portrait, rules.portraits_for(self.race, self.gender).len(), delta);
    }

    pub fn step_background(&mut self, rules: &CreationRules, delta: i32) {
        Self::step(&mut self.background, rules.backgrounds_for(self.race, self.gender).len(), delta);
    }

    /// Adds a typed character if it fits and the game's fonts can show it.
    pub fn type_char(&mut self, c: char) {
        if self.name.chars().count() < MAX_NAME && !c.is_control() && arcanum_formats::cp1252::encode_char(c).is_some() {
            self.name.push(c);
        }
    }

    /// Character points left to spend.
    pub fn unspent(&self) -> i32 {
        STARTING_POINTS - (self.bought.iter().sum::<i32>() + self.hp_bought + self.fatigue_bought) * STAT_COST
    }

    /// Buys or sells a step of hit points (`hp`) or fatigue.
    pub fn adjust_pool(&mut self, hp: bool, delta: i32) -> Result<(), EditError> {
        let unspent = self.unspent();
        let pool = if hp { &mut self.hp_bought } else { &mut self.fatigue_bought };
        if delta > 0 {
            if unspent < STAT_COST {
                return Err(EditError::NoPoints);
            }
            *pool += 1;
        } else {
            if *pool == 0 {
                return Err(EditError::BelowAccepted);
            }
            *pool -= 1;
        }
        Ok(())
    }

    /// A name the player has actually entered.
    pub fn has_name(&self) -> bool {
        !self.name.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mes(text: &str) -> MesFile {
        MesFile::parse(text.as_bytes()).unwrap()
    }

    fn rules() -> CreationRules {
        CreationRules::from_files(
            &mes("{0}{1000}{1}{86}{2}{}{10}{1001}{11}{87}{12}{}{30}{1003}{31}{89}{32}{HUF HUM}{390}{1039}{392}{NPC}{40}{1004}{41}{90}{42}{DWM}"),
            &mes("{65}{st +1, cn +1, ch -1, dx -1, repair +2}{87}{resistpoison +20, be -1}{89}{repair -1}{90}{pe +1}"),
            &mes("{1000}{ELM1}{1001}{DWM1}{1005}{HUM1}{1018}{HUM2}{1020}{DWM2}{1008}{HUF1}{1035}{NPCDoc}"),
        )
    }

    #[test]
    fn backgrounds_follow_the_legal_codes_and_skip_npc_ones() {
        let r = rules();
        let ids = |race, gender| r.backgrounds_for(race, gender).iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(0, MALE), [0, 1, 3], "human male: no restriction, HUM");
        assert_eq!(ids(1, MALE), [0, 1, 4], "dwarf male: DWM only");
        assert!(!r.backgrounds.iter().any(|b| b.id == 39), "pre-made (NPC) backgrounds are never offered");
    }

    #[test]
    fn portraits_match_the_race_and_gender_code() {
        let r = rules();
        assert_eq!(r.portraits_for(0, MALE), ["HUM1", "HUM2"]);
        assert_eq!(r.portraits_for(1, MALE), ["DWM1", "DWM2"]);
        assert_eq!(r.portraits_for(0, FEMALE), ["HUF1"]);
    }

    #[test]
    fn stats_add_race_and_background_effects() {
        let r = rules();
        let mut c = NewCharacter { race: 1, ..NewCharacter::default() };
        assert_eq!(r.stats(&c), [9, 7, 9, 8, 8, 8, 8, 7], "dwarf: st +1 cn +1 ch -1 dx -1");
        c.background = 2; // DWM-only background, pe +1
        assert_eq!(r.background(&c).unwrap().id, 4);
        assert_eq!(r.stats(&c)[5], 9);
        assert_eq!(parse_stat_effect("wp +1, dx +1, be +1, cn -2, st -1, magicpts +3"), [-1, 1, -2, 1, 0, 0, 1, 0]);
    }

    #[test]
    fn race_skips_male_only_races_for_women_and_resets_choices() {
        let mut c = NewCharacter { gender: FEMALE, portrait: 1, background: 2, ..NewCharacter::default() };
        c.step_race(1);
        assert_eq!(c.race, 2, "dwarf has no women: straight to elf");
        assert_eq!((c.portrait, c.background), (0, 0));
        c.step_race(1);
        assert_eq!(c.race, 3);
        c.step_race(1);
        assert_eq!(c.race, 6, "half-orc after half-elf");
        c.step_race(1);
        assert_eq!(c.race, 0, "wraps past half-ogre");
        c.step_race(-1);
        assert_eq!(c.race, 6);
    }

    #[test]
    fn gender_only_switches_where_the_race_allows_it() {
        let mut c = NewCharacter { race: 1, ..NewCharacter::default() };
        assert!(!c.toggle_gender(), "dwarves are male only");
        assert_eq!(c.gender, MALE);
        c.race = 2;
        assert!(c.toggle_gender());
        assert_eq!(c.gender, FEMALE);
    }

    #[test]
    fn portraits_and_backgrounds_wrap() {
        let r = rules();
        let mut c = NewCharacter::default();
        c.step_portrait(&r, -1);
        assert_eq!(r.portrait(&c), Some("HUM2"));
        c.step_portrait(&r, 1);
        assert_eq!(r.portrait(&c), Some("HUM1"));
        c.step_background(&r, -1);
        assert_eq!(r.background(&c).unwrap().id, 3);
    }

    #[test]
    fn names_are_limited_and_printable() {
        let mut c = NewCharacter::default();
        assert!(!c.has_name());
        for ch in "Virgil the very long-named hero".chars() {
            c.type_char(ch);
        }
        assert_eq!(c.name.chars().count(), MAX_NAME);
        let mut d = NewCharacter::default();
        d.type_char('\u{8}');
        d.type_char('日');
        d.type_char(' ');
        assert!(!d.has_name(), "control, unsupported and blank input do not make a name");
    }

    #[test]
    fn spending_points_follows_the_original_rules() {
        let r = rules();
        let mut c = NewCharacter::default();
        assert_eq!(c.unspent(), STARTING_POINTS);
        r.raise_stat(&mut c, 0).unwrap();
        assert_eq!((r.stats(&c)[0], c.unspent()), (9, 4));
        assert_eq!(r.lower_stat(&mut c, 1), Err(EditError::BelowAccepted), "not below the starting value");
        r.lower_stat(&mut c, 0).unwrap();
        assert_eq!(c.unspent(), STARTING_POINTS);
        for _ in 0..5 {
            r.raise_stat(&mut c, 7).unwrap();
        }
        assert_eq!(r.raise_stat(&mut c, 7), Err(EditError::NoPoints));
        assert_eq!(c.adjust_pool(true, 1), Err(EditError::NoPoints));
        // The race maximum stops a stat even with points left.
        let mut d = NewCharacter { bought: [12, 0, 0, 0, 0, 0, 0, 0], ..NewCharacter::default() };
        assert_eq!(r.stats(&d)[0], 20);
        assert_eq!(r.raise_stat(&mut d, 0), Err(EditError::AtMaximum));
        assert_eq!(race_max(7, 0), 24, "half-ogres reach 24 strength");
    }

    #[test]
    fn derived_stats_use_the_original_formulas() {
        let r = rules();
        let c = NewCharacter::default();
        let d = r.derived(&c);
        assert_eq!((d.carry_weight, d.damage_bonus, d.ac_adjustment, d.speed), (4000, -1, -2, 8));
        assert_eq!((d.heal_rate, d.poison_recovery, d.reaction, d.max_followers), (3, 8, -7, 2));
        // HP = 4 x bought + WP + 2 x (level + ST) + 4 = 0 + 8 + 18 + 4.
        assert_eq!((d.hit_points, d.fatigue), (30, 30));
        let mut e = NewCharacter::default();
        e.adjust_pool(true, 1).unwrap();
        assert_eq!(r.derived(&e).hit_points, 34);
        assert_eq!(reaction_modifier(10), 0);
        assert_eq!(reaction_modifier(15), 25);
        assert_eq!(reaction_modifier(1), -65);
        assert_eq!(reaction_modifier(22), 120);
    }

    #[test]
    fn effects_carry_resistances() {
        let e = parse_effect("st +1, cn +1, melee +2, resistpoison +10, be -2");
        assert_eq!(e.stats, [1, 0, 1, -2, 0, 0, 0, 0]);
        assert_eq!(e.resist, [0, 0, 0, 10, 0]);
    }
}
