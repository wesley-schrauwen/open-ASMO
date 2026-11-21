//! The character sheet, its draft, and why a change can be refused.

use crate::tables::{COLLEGES, DISCIPLINES, SKILLS};

/// The eight primary stats in `mes/stat.mes` order: ST DX CN BE IN PE WP CH.
pub const PRIMARY_STATS: usize = 8;
/// Every primary stat of a new character before race and background (template 016086).
pub const BASE_STAT: i32 = 8;
/// Points a new level-1 character starts with (stat 21 in the prototypes).
pub const STARTING_POINTS: i32 = 5;
/// Character points per step of a primary stat, hit points or fatigue (table 0x5b52e0: flat 1).
pub const STAT_COST: i32 = 1;
/// No stat goes below this (table 0x5b5194).
pub const MIN_STAT: i32 = 1;
/// Longest name: the original's buffer holds 23 bytes including the terminator (0x5461a0).
pub const MAX_NAME: usize = 22;
pub const RACE_COUNT: usize = 8;
/// Which races allow a female character (Arcanum.exe race table 0x5c5170).
pub const FEMALE_ALLOWED: [bool; RACE_COUNT] = [true, false, true, true, false, false, true, false];
pub const FEMALE: u8 = 0;
pub const MALE: u8 = 1;

/// The two pools bought with character points (+4 each per point).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Pool {
    HitPoints,
    Fatigue,
}

/// Changes since the last accept. Only these can be sold back ("cannot reduce below its last
/// accepted level", `mes/charedit.mes` 19).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Draft {
    pub stats: [i32; PRIMARY_STATS],
    pub hit_points: i32,
    pub fatigue: i32,
    /// Skill points bought, basic skills then tech skills (`mes/skill.mes` order).
    #[cfg_attr(feature = "serde", serde(default))]
    pub skills: [i32; SKILLS],
    /// Tech degrees bought per discipline.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tech: [i32; DISCIPLINES],
    /// Spells bought per college.
    #[cfg_attr(feature = "serde", serde(default))]
    pub spells: [i32; COLLEGES],
}

impl Draft {
    /// Character points the draft spends: 1 a stat step, pool step, skill point (0x4c64b0),
    /// tech degree (0x5b5124) or spell (0x4b1650).
    pub fn spent(&self) -> i32 {
        let sum = |v: &[i32]| v.iter().sum::<i32>();
        (sum(&self.stats) + self.hit_points + self.fatigue + sum(&self.skills) + sum(&self.tech) + sum(&self.spells)) * STAT_COST
    }
}

/// One character: accepted values plus a draft of unaccepted changes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CharacterSheet {
    pub name: String,
    pub race: u8,
    pub gender: u8,
    pub age: i32,
    pub level: i32,
    /// `Rules/backgrnd.mes` block id (the block starts at `id * 10`).
    pub background: u32,
    /// `portrait/gameport.mes` id; `None` = the first portrait for the race and gender.
    pub portrait: Option<u32>,
    /// Accepted base stats, without race or background effects.
    pub base: [i32; PRIMARY_STATS],
    /// Accepted points in each pool.
    pub hp_points: i32,
    pub fatigue_points: i32,
    /// Accepted unspent character points.
    pub unspent: i32,
    /// Accepted skill points (0-5 each), basic skills then tech skills (fields 0xdd/0xde, bits
    /// 0-5).
    #[cfg_attr(feature = "serde", serde(default))]
    pub skills: [i32; SKILLS],
    /// Training rank per skill: 0 none, 1 Apprentice, 2 Expert, 3 Master (bits 6-7). Only
    /// trainers change it, never the editor.
    #[cfg_attr(feature = "serde", serde(default))]
    pub training: [u8; SKILLS],
    /// Accepted degree per tech discipline, 0 (Layman) to 7 (Doctorate) (field 0xdf, 17-24).
    #[cfg_attr(feature = "serde", serde(default))]
    pub tech: [i32; DISCIPLINES],
    /// Accepted spells known per college, 0-5: always the college's first ones (field 0xdf,
    /// 0-15).
    #[cfg_attr(feature = "serde", serde(default))]
    pub spells: [i32; COLLEGES],
    pub draft: Draft,
}

impl Default for CharacterSheet {
    /// As the "create your own" template (prototype 016086): a human male, level 1, first
    /// portrait, background 0, no name, 5 points.
    fn default() -> Self {
        Self {
            name: String::new(),
            race: 0,
            gender: MALE,
            age: 0,
            level: 1,
            background: 0,
            portrait: None,
            base: [BASE_STAT; PRIMARY_STATS],
            hp_points: 0,
            fatigue_points: 0,
            unspent: STARTING_POINTS,
            skills: [0; SKILLS],
            training: [0; SKILLS],
            tech: [0; DISCIPLINES],
            spells: [0; COLLEGES],
            draft: Draft::default(),
        }
    }
}

impl CharacterSheet {
    /// A name the player has actually entered.
    pub fn has_name(&self) -> bool {
        !self.name.trim().is_empty()
    }
}

/// Which text file a refusal's message is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// `mes/charedit.mes`
    Charedit(u32),
    /// `mes/MainMenu.mes`
    MainMenu(u32),
}

/// Why a change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Refusal {
    /// charedit 17 "Stat has reached its maximum value."
    AtMaximum,
    /// charedit 18 "You haven't enough character points."
    NoPoints,
    /// charedit 19 "You cannot reduce this stat below its last accepted level."
    BelowAccepted,
    /// charedit 20 "Stat has reached its minimum value."
    AtMinimum,
    /// charedit 21 "Skill has reached its maximum value."
    SkillAtMaximum,
    /// charedit 22-38 "You must raise your <stat> first.": the governing stat caps the skill.
    RaiseStatFirst(u8),
    /// charedit 23 "You cannot reduce this skill below its last accepted level."
    SkillBelowAccepted,
    /// charedit 24 "You have no points in this skill."
    NoSkillPoints,
    /// charedit 28: lowering the stat would leave a skill above what it allows.
    SkillPrerequisite,
    /// charedit 27 "You cannot sell back an accepted spell."
    SpellAccepted,
    /// charedit 29: lowering Intelligence below what a known spell needs.
    SpellNeedsIntelligence,
    /// charedit 39: lowering Willpower below what a known spell needs.
    SpellNeedsWillpower,
    /// charedit 30 "You are not high enough level to buy this spell."
    SpellLevel,
    /// charedit 31: lowering Intelligence below what a tech degree needs.
    TechNeedsIntelligence,
    /// The original ignores the click without a message (e.g. female for a male-only race).
    NotAllowed,
    /// MainMenu 506: Next without a name.
    NoName,
    /// A pre-made character that is not in the data.
    UnknownPremade,
}

impl Refusal {
    /// The message the original shows, if any.
    pub fn message(self) -> Option<Message> {
        match self {
            Self::AtMaximum => Some(Message::Charedit(17)),
            Self::NoPoints => Some(Message::Charedit(18)),
            Self::BelowAccepted => Some(Message::Charedit(19)),
            Self::AtMinimum => Some(Message::Charedit(20)),
            Self::SkillAtMaximum => Some(Message::Charedit(21)),
            Self::RaiseStatFirst(stat) => Some(Message::Charedit(raise_first_message(usize::from(stat)))),
            Self::SkillBelowAccepted => Some(Message::Charedit(23)),
            Self::NoSkillPoints => Some(Message::Charedit(24)),
            Self::SkillPrerequisite => Some(Message::Charedit(28)),
            Self::SpellAccepted => Some(Message::Charedit(27)),
            Self::SpellNeedsIntelligence => Some(Message::Charedit(29)),
            Self::SpellNeedsWillpower => Some(Message::Charedit(39)),
            Self::SpellLevel => Some(Message::Charedit(30)),
            Self::TechNeedsIntelligence => Some(Message::Charedit(31)),
            Self::NoName => Some(Message::MainMenu(506)),
            Self::NotAllowed | Self::UnknownPremade => None,
        }
    }
}

/// The "You must raise your <stat> first." message for a primary stat (Arcanum.exe 0x55f200:
/// IN is 26, the others 32-38 in the order ST DX CN BE WP PE CH; 22 "your stats" otherwise).
pub fn raise_first_message(stat: usize) -> u32 {
    match stat {
        0 => 32,
        1 => 33,
        2 => 34,
        3 => 35,
        4 => 26,
        5 => 37,
        6 => 36,
        7 => 38,
        _ => 22,
    }
}
