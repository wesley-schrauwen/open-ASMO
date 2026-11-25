//! One enum for every player change to a character, applied by one function. The client applies
//! an action to preview it; the host applies the same action and its result counts.

use crate::*;

/// A player's change to a character.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CharacterAction {
    StepRace(i32),
    ToggleGender,
    StepPortrait(i32),
    StepBackground(i32),
    SetName(String),
    RaiseStat(usize),
    LowerStat(usize),
    AdjustPool(Pool, i32),
    /// A point into or out of a skill (`mes/skill.mes` index: basic skills, then tech skills).
    RaiseSkill(usize),
    LowerSkill(usize),
    /// A degree in a tech discipline (`mes/tech.mes` 0-7).
    RaiseTech(usize),
    LowerTech(usize),
    /// The next spell of a college (`mes/spell.mes` 500-515), or the top one back.
    RaiseSpell(usize),
    LowerSpell(usize),
    /// Take a pre-made character (prototype number) instead.
    PickPremade(u32),
    Accept,
}

impl RulesData {
    pub fn apply(&self, s: &CharacterSheet, action: &CharacterAction) -> Result<CharacterSheet, Refusal> {
        match action {
            CharacterAction::StepRace(d) => Ok(self.step_race(s, *d)),
            CharacterAction::ToggleGender => self.toggle_gender(s),
            CharacterAction::StepPortrait(d) => Ok(self.step_portrait(s, *d)),
            CharacterAction::StepBackground(d) => Ok(self.step_background(s, *d)),
            CharacterAction::SetName(name) => Ok(self.set_name(s, name)),
            CharacterAction::RaiseStat(i) if *i < PRIMARY_STATS => self.raise_stat(s, *i),
            CharacterAction::LowerStat(i) if *i < PRIMARY_STATS => self.lower_stat(s, *i),
            CharacterAction::RaiseStat(_) | CharacterAction::LowerStat(_) => Err(Refusal::NotAllowed),
            CharacterAction::AdjustPool(pool, d) => self.adjust_pool(s, *pool, *d),
            CharacterAction::RaiseSkill(k) if *k < SKILLS => self.raise_skill(s, *k),
            CharacterAction::LowerSkill(k) if *k < SKILLS => self.lower_skill(s, *k),
            CharacterAction::RaiseSkill(_) | CharacterAction::LowerSkill(_) => Err(Refusal::NotAllowed),
            CharacterAction::RaiseTech(d) => self.raise_tech(s, *d),
            CharacterAction::LowerTech(d) => self.lower_tech(s, *d),
            CharacterAction::RaiseSpell(c) => self.raise_spell(s, *c),
            CharacterAction::LowerSpell(c) => self.lower_spell(s, *c),
            CharacterAction::PickPremade(p) => {
                self.premades.iter().find(|x| x.prototype == *p).map(|x| x.sheet.clone()).ok_or(Refusal::UnknownPremade)
            }
            CharacterAction::Accept => Ok(self.accept(s)),
        }
    }
}
