//! Skills: 12 basic skills and 4 tech skills, bought with character points (Arcanum.exe; see
//! NOTES.md "Character rules").
//!
//! - A skill holds 0-5 points; its level is `points x 4` plus race and background effects,
//!   clamped to 0-20, then capped by its governing stat ([`skill_cap`], 0x4c5eb0 / 0x4c65e0).
//! - A point costs one character point (0x4c64b0). Raising is refused without points (18), at
//!   level 20 (21), or when the governing stat's cap is below the new `points x 4` ("raise your
//!   <stat> first", 0x57acd0).
//! - Lowering gives back one point, down to the accepted points only (23; 24 with none). The
//!   original's basic skill minus gives back points until the level changes (0x55d640); that
//!   only differs when a point above level 20 was bought, which raising refuses.
//! - Training ranks are shown, never bought here.

use crate::RulesData;
use crate::sheet::{CharacterSheet, PRIMARY_STATS, Refusal, STAT_COST};
use crate::tables::SKILLS;

/// Each skill's governing primary stat (tables 0x5b6f04 / 0x5b6f34; ST DX CN BE IN PE WP CH).
pub const SKILL_STAT: [usize; SKILLS] = [1, 1, 1, 1, 1, 1, 5, 5, 4, 6, 4, 7, 4, 5, 1, 5];
/// Skill level per point.
pub const LEVELS_PER_POINT: i32 = 4;
pub const MAX_SKILL_LEVEL: i32 = 20;
/// Skills per group on the character screen: combat, thieving, social, technological.
pub const SKILL_GROUP_SIZE: usize = 4;

/// The highest level a governing stat allows (table 0x5b6fa0).
pub fn skill_cap(stat: i32) -> i32 {
    match stat {
        i32::MIN..=5 => 3,
        6..=8 => 7,
        9..=11 => 11,
        12..=14 => 15,
        15..=17 => 19,
        _ => 20,
    }
}

/// Points in skill `k`, accepted and in the draft.
pub fn skill_points(s: &CharacterSheet, k: usize) -> i32 {
    s.skills[k] + s.draft.skills[k]
}

impl RulesData {
    /// Skill `k`'s level, 0-20.
    pub fn skill_level(&self, s: &CharacterSheet, k: usize) -> i32 {
        let raw = self.apply_effects(
            crate::EffectTarget::Skill(k as u8),
            skill_points(s, k) * LEVELS_PER_POINT,
            &self.effect_ids(s),
        );
        let stat = self.stats(s)[SKILL_STAT[k]];
        raw.clamp(0, MAX_SKILL_LEVEL).min(skill_cap(stat))
    }

    /// Buys a point in skill `k`.
    pub fn raise_skill(&self, s: &CharacterSheet, k: usize) -> Result<CharacterSheet, Refusal> {
        if self.unspent(s) < STAT_COST {
            return Err(Refusal::NoPoints);
        }
        if self.skill_level(s, k) >= MAX_SKILL_LEVEL {
            return Err(Refusal::SkillAtMaximum);
        }
        let stat = SKILL_STAT[k];
        if skill_cap(self.stats(s)[stat]) < (skill_points(s, k) + 1) * LEVELS_PER_POINT {
            debug_assert!(stat < PRIMARY_STATS);
            return Err(Refusal::RaiseStatFirst(stat as u8));
        }
        let mut out = s.clone();
        out.draft.skills[k] += 1;
        Ok(out)
    }

    /// Sells back points in skill `k` bought since the last accept.
    pub fn lower_skill(&self, s: &CharacterSheet, k: usize) -> Result<CharacterSheet, Refusal> {
        if s.draft.skills[k] == 0 {
            return Err(if skill_points(s, k) == 0 {
                Refusal::NoSkillPoints
            } else {
                Refusal::SkillBelowAccepted
            });
        }
        let mut out = s.clone();
        out.draft.skills[k] -= 1;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcanum_formats::mes::MesFile;

    fn mes(text: &str) -> MesFile {
        MesFile::parse(text.as_bytes()).unwrap()
    }

    fn rules() -> RulesData {
        // Background 1: effect 86, melee +2; race human (64) has nothing.
        RulesData::from_files(
            &mes("{0}{1000}{1}{}{2}{}{10}{1001}{11}{86}{12}{}"),
            &mes("{86}{Melee +2, dx +4}"),
            &mes("{1005}{HUM1}"),
        )
    }

    fn with(points: i32) -> CharacterSheet {
        CharacterSheet {
            unspent: points,
            ..rules().new_sheet()
        }
    }

    #[test]
    fn the_governing_stat_caps_the_skill() {
        assert_eq!(
            [1, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18, 20, 24].map(skill_cap),
            [3, 3, 7, 7, 11, 11, 15, 15, 19, 19, 20, 20, 20]
        );
        let r = rules();
        let s = r.raise_skill(&with(5), 0).unwrap();
        assert_eq!(
            (r.skill_level(&s, 0), r.unspent(&s)),
            (4, 4),
            "4 levels a point, 1 character point"
        );
        // DX 8 allows level 7: a second point would need 8.
        assert_eq!(r.raise_skill(&s, 0), Err(Refusal::RaiseStatFirst(1)));
        assert_eq!(
            Refusal::RaiseStatFirst(1).message(),
            Some(crate::Message::Charedit(33))
        );
        assert_eq!(
            Refusal::RaiseStatFirst(4).message(),
            Some(crate::Message::Charedit(26)),
            "IN is 26"
        );
        // Raising DX lifts the cap.
        let s = r.raise_stat(&r.raise_stat(&s, 1).unwrap(), 1).unwrap();
        let s = r.raise_skill(&s, 0).unwrap();
        assert_eq!(r.skill_level(&s, 0), 8);
        assert_eq!(
            r.raise_skill(&s, 0),
            Err(Refusal::RaiseStatFirst(1)),
            "DX 10 allows 11"
        );
        let s = r.raise_skill(&s, 1).unwrap();
        assert_eq!(
            r.raise_skill(&s, 4),
            Err(Refusal::NoPoints),
            "checked first"
        );
    }

    #[test]
    fn a_stat_cannot_drop_below_what_a_skill_needs() {
        let r = rules();
        let s = r.raise_stat(&with(5), 1).unwrap(); // DX 9: cap 11
        let s = r.raise_skill(&r.raise_skill(&s, 3).unwrap(), 3).unwrap(); // Throwing 8
        assert_eq!(r.lower_stat(&s, 1), Err(Refusal::SkillPrerequisite));
        let s = r.lower_skill(&s, 3).unwrap();
        assert!(r.lower_stat(&s, 1).is_ok());
    }

    #[test]
    fn lowering_goes_back_to_the_accepted_points() {
        let r = rules();
        let s = with(5);
        assert_eq!(r.lower_skill(&s, 2), Err(Refusal::NoSkillPoints));
        let s = r.accept(&r.raise_skill(&s, 2).unwrap());
        assert_eq!((s.skills[2], s.unspent), (1, 4));
        assert_eq!(r.lower_skill(&s, 2), Err(Refusal::SkillBelowAccepted));
    }

    #[test]
    fn effects_add_levels_and_count_toward_twenty() {
        let r = rules();
        // Background 1: Melee +2 and DX 12 (cap 15).
        let s = CharacterSheet {
            background: 1,
            skills: [0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ..with(5)
        };
        assert_eq!(r.skill_level(&s, 2), 14, "3 points x 4 + 2");
        let strong = CharacterSheet {
            base: [8, 20, 8, 8, 8, 8, 8, 8],
            ..s.clone()
        };
        let s = r
            .raise_skill(&r.raise_skill(&strong, 2).unwrap(), 2)
            .unwrap();
        assert_eq!(r.skill_level(&s, 2), 20);
        assert_eq!(
            r.raise_skill(&s, 2),
            Err(Refusal::SkillAtMaximum),
            "level 20 with 5 points"
        );
        let s = r.lower_skill(&s, 2).unwrap();
        assert_eq!((s.draft.skills[2], r.skill_level(&s, 2)), (1, 18));
    }
}
