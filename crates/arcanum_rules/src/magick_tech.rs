//! Tech degrees and spells, bought with character points (Arcanum.exe; see NOTES.md
//! "Character rules").
//!
//! Tech: each of the 8 disciplines has a degree from 0 (Layman) to 7 (Doctorate).
//! - A degree costs one point (table 0x5b5124) and needs Intelligence
//!   `[0, 5, 8, 11, 13, 15, 17, 19][degree]` (0x5b5164); without it, or at degree 7, the
//!   editor says "You must raise your Intelligence first." (26).
//! - Minus sells the top degree back, down to the degree the editor opened with: the screen
//!   only shows the minus button above it in game (0x55c3a0). (The rule itself, 0x4b02b0,
//!   would sell accepted degrees too.)
//!
//! Magick: each of the 16 colleges has 5 spells, known in order.
//! - Only the next spell of a college can be bought and only the top one sold (0x4b1790,
//!   0x4b19b0); one point each (0x4b1650).
//! - The spell of rank `r` (0-4 in its college) needs level `[1, 1, 5, 10, 15][r]` (30),
//!   Intelligence 5 (26) and Willpower `[6, 9, 12, 15, 18][r]` (36), checked after the points
//!   (18) in that order (0x55f160). A full college refuses with 36, the original's fallback.
//! - Accepted spells cannot be sold back (27).
//! - Mastery of a college is earned in play, never here.

use crate::RulesData;
use crate::sheet::{CharacterSheet, Refusal, STAT_COST};
use crate::tables::{COLLEGES, DISCIPLINES, SPELLS_PER_COLLEGE};

pub const INTELLIGENCE: usize = 4;
pub const WILLPOWER: usize = 6;
pub const MAX_DEGREE: i32 = 7;
/// Intelligence each degree needs (0x5b5164).
const DEGREE_INTELLIGENCE: [i32; 8] = [0, 5, 8, 11, 13, 15, 17, 19];
/// Intelligence every spell needs (the data has no `IQ:` overrides).
pub const SPELL_INTELLIGENCE: i32 = 5;
/// Level and Willpower each spell rank needs (0x5b5770, 0x5b0de0).
const SPELL_LEVEL: [i32; SPELLS_PER_COLLEGE] = [1, 1, 5, 10, 15];
const SPELL_WILLPOWER: [i32; SPELLS_PER_COLLEGE] = [6, 9, 12, 15, 18];

/// The Intelligence a tech degree needs.
pub fn degree_intelligence(degree: i32) -> i32 {
    DEGREE_INTELLIGENCE[degree.clamp(0, MAX_DEGREE) as usize]
}

/// Discipline `d`'s degree, accepted plus draft.
pub fn tech_degree(s: &CharacterSheet, d: usize) -> i32 {
    s.tech[d] + s.draft.tech[d]
}

/// Spells known per college, accepted plus draft.
pub fn spells_known(s: &CharacterSheet) -> [i32; COLLEGES] {
    std::array::from_fn(|c| s.spells[c] + s.draft.spells[c])
}

/// The Willpower the known spells need (0 with none).
pub fn spell_willpower_needed(s: &CharacterSheet) -> i32 {
    spells_known(s).iter().filter(|&&n| n > 0).map(|&n| SPELL_WILLPOWER[(n as usize - 1).min(SPELLS_PER_COLLEGE - 1)]).max().unwrap_or(0)
}

/// Magick/tech aptitude limits: +100 is all magick, -100 all tech.
pub const MAX_APTITUDE: i32 = 100;
/// Alignment limits: -1000 evil to +1000 good (stat 19).
pub const MAX_ALIGNMENT: i32 = 1000;

impl RulesData {
    /// The magick/tech aptitude (stat 16, 0x4b08ce): `(50 x spells - 55 x degrees) / 10`, with
    /// `magicpts`/`techpts` effects counted as spells and degrees, plus `aptitude` effects,
    /// clamped to -100..100. The sector's aura (sector + 0x4644) is not built.
    pub fn aptitude(&self, s: &CharacterSheet) -> i32 {
        let e = self.character_effects(s);
        let spells: i32 = spells_known(s).iter().sum::<i32>() + e.magick_points;
        let degrees: i32 = (0..DISCIPLINES).map(|d| tech_degree(s, d)).sum::<i32>() + e.tech_points;
        ((50 * spells - 55 * degrees) / 10 + e.aptitude).clamp(-MAX_APTITUDE, MAX_APTITUDE)
    }

    /// The alignment (stat 19): 0 plus `alignment` effects, clamped to -1000..1000. (A new
    /// character's starting value was not traced; nothing changes it in play yet.)
    pub fn alignment(&self, s: &CharacterSheet) -> i32 {
        self.character_effects(s).alignment.clamp(-MAX_ALIGNMENT, MAX_ALIGNMENT)
    }

    /// Raises discipline `d` by a degree.
    pub fn raise_tech(&self, s: &CharacterSheet, d: usize) -> Result<CharacterSheet, Refusal> {
        if d >= DISCIPLINES {
            return Err(Refusal::NotAllowed);
        }
        if self.unspent(s) < STAT_COST {
            return Err(Refusal::NoPoints);
        }
        let next = tech_degree(s, d) + 1;
        if next > MAX_DEGREE || self.stats(s)[INTELLIGENCE] < degree_intelligence(next) {
            return Err(Refusal::RaiseStatFirst(INTELLIGENCE as u8));
        }
        let mut out = s.clone();
        out.draft.tech[d] += 1;
        Ok(out)
    }

    /// Sells discipline `d`'s top degree back, if bought since the last accept.
    pub fn lower_tech(&self, s: &CharacterSheet, d: usize) -> Result<CharacterSheet, Refusal> {
        if d >= DISCIPLINES {
            return Err(Refusal::NotAllowed);
        }
        if s.draft.tech[d] <= 0 {
            return Err(Refusal::NotAllowed);
        }
        let mut out = s.clone();
        out.draft.tech[d] -= 1;
        Ok(out)
    }

    /// Buys the next spell of college `c`.
    pub fn raise_spell(&self, s: &CharacterSheet, c: usize) -> Result<CharacterSheet, Refusal> {
        if c >= COLLEGES {
            return Err(Refusal::NotAllowed);
        }
        if self.unspent(s) < STAT_COST {
            return Err(Refusal::NoPoints);
        }
        let rank = spells_known(s)[c] as usize;
        if rank >= SPELLS_PER_COLLEGE {
            return Err(Refusal::RaiseStatFirst(WILLPOWER as u8));
        }
        let stats = self.stats(s);
        if s.level < SPELL_LEVEL[rank] {
            return Err(Refusal::SpellLevel);
        }
        if stats[INTELLIGENCE] < SPELL_INTELLIGENCE {
            return Err(Refusal::RaiseStatFirst(INTELLIGENCE as u8));
        }
        if stats[WILLPOWER] < SPELL_WILLPOWER[rank] {
            return Err(Refusal::RaiseStatFirst(WILLPOWER as u8));
        }
        let mut out = s.clone();
        out.draft.spells[c] += 1;
        Ok(out)
    }

    /// Sells college `c`'s top spell back, if bought since the last accept.
    pub fn lower_spell(&self, s: &CharacterSheet, c: usize) -> Result<CharacterSheet, Refusal> {
        if c >= COLLEGES {
            return Err(Refusal::NotAllowed);
        }
        if s.draft.spells[c] == 0 {
            return Err(if s.spells[c] > 0 { Refusal::SpellAccepted } else { Refusal::NotAllowed });
        }
        let mut out = s.clone();
        out.draft.spells[c] -= 1;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::Message;

    fn sheet(points: i32, base: [i32; 8]) -> CharacterSheet {
        CharacterSheet { unspent: points, base, ..RulesData::default().new_sheet() }
    }

    #[test]
    fn degrees_need_intelligence_and_go_back_to_the_opening_degree() {
        let r = RulesData::default();
        let s = r.raise_tech(&sheet(5, [8; 8]), 2).unwrap();
        assert_eq!((tech_degree(&s, 2), r.unspent(&s)), (1, 4));
        let s = r.raise_tech(&s, 2).unwrap();
        assert_eq!(r.raise_tech(&s, 2), Err(Refusal::RaiseStatFirst(4)), "IN 8 allows degree 2, not 3 (11)");
        let sold = r.lower_tech(&s, 2).unwrap();
        assert_eq!((tech_degree(&sold, 2), r.unspent(&sold)), (1, 4));
        assert_eq!(r.lower_tech(&r.accept(&s), 2), Err(Refusal::NotAllowed), "not below the opening degree");
        let doctor = CharacterSheet { tech: [7, 0, 0, 0, 0, 0, 0, 0], ..sheet(5, [20; 8]) };
        assert_eq!(r.raise_tech(&doctor, 0), Err(Refusal::RaiseStatFirst(4)), "degree 7 says raise IN, as the original");
    }

    #[test]
    fn the_next_spell_needs_level_intelligence_and_willpower() {
        let r = RulesData::default();
        let s = sheet(5, [8, 8, 8, 8, 8, 8, 9, 8]);
        let s = r.raise_spell(&s, 3).unwrap();
        let s = r.raise_spell(&s, 3).unwrap();
        assert_eq!(spells_known(&s)[3], 2);
        assert_eq!(r.raise_spell(&s, 3), Err(Refusal::SpellLevel), "rank 2 needs level 5");
        let dull = sheet(5, [8, 8, 8, 8, 4, 8, 9, 8]);
        assert_eq!(r.raise_spell(&dull, 0), Err(Refusal::RaiseStatFirst(4)));
        let weak = sheet(5, [8, 8, 8, 8, 8, 8, 5, 8]);
        assert_eq!(r.raise_spell(&weak, 0), Err(Refusal::RaiseStatFirst(6)));
        assert_eq!(Refusal::RaiseStatFirst(6).message(), Some(Message::Charedit(36)));
        // Only the top one goes back, and only while not accepted.
        let s = r.lower_spell(&s, 3).unwrap();
        let s = r.accept(&s);
        assert_eq!(r.lower_spell(&s, 3), Err(Refusal::SpellAccepted));
        assert_eq!(r.lower_spell(&s, 4), Err(Refusal::NotAllowed));
    }

    #[test]
    fn aptitude_leans_to_magick_with_spells_and_to_tech_with_degrees() {
        use arcanum_formats::mes::MesFile;
        let mes = |t: &str| MesFile::parse(t.as_bytes()).unwrap();
        // Dwarves (race effect 65) count three tech degrees; background 1 is evil.
        let r = RulesData::from_files(&mes("{0}{1000}{1}{}{2}{}{10}{1001}{11}{86}{12}{}"), &mes("{65}{techpts +3}{86}{alignment -50, alignment max -200}"), &mes("{1001}{DWM1}"));
        let human = sheet(5, [8, 8, 8, 8, 8, 8, 9, 8]);
        assert_eq!(r.aptitude(&human), 0);
        let mage = r.raise_spell(&human, 0).unwrap();
        assert_eq!(r.aptitude(&mage), 5, "50 / 10");
        let both = r.raise_tech(&mage, 0).unwrap();
        assert_eq!(r.aptitude(&both), 0, "(50 - 55) / 10 rounds towards zero");
        let dwarf = CharacterSheet { race: 1, ..human.clone() };
        assert_eq!(r.aptitude(&dwarf), -16, "-165 / 10");
        let full = CharacterSheet { spells: [5; 16], ..human.clone() };
        assert_eq!(r.aptitude(&full), MAX_APTITUDE, "clamped");
        assert_eq!(r.alignment(&CharacterSheet { background: 1, ..human }), -50);
    }

    #[test]
    fn stats_cannot_drop_below_what_spells_and_degrees_need() {
        let r = RulesData::default();
        // WP 9 bought from 8: the second spell needs 9.
        let s = r.raise_stat(&sheet(5, [8; 8]), WILLPOWER).unwrap();
        let s = r.raise_spell(&r.raise_spell(&s, 0).unwrap(), 0).unwrap();
        assert_eq!(r.lower_stat(&s, WILLPOWER), Err(Refusal::SpellNeedsWillpower));
        // IN 8 bought from 7: degree 2 needs 8.
        let s = r.raise_stat(&sheet(5, [8, 8, 8, 8, 7, 8, 8, 8]), INTELLIGENCE).unwrap();
        let s = r.raise_tech(&r.raise_tech(&s, 0).unwrap(), 0).unwrap();
        assert_eq!(r.lower_stat(&s, INTELLIGENCE), Err(Refusal::TechNeedsIntelligence));
        // Spells check IN 5 first: IN 5 bought from 4 with a spell known.
        let s = r.raise_stat(&sheet(5, [8, 8, 8, 8, 4, 8, 8, 8]), INTELLIGENCE).unwrap();
        let s = r.raise_spell(&s, 0).unwrap();
        assert_eq!(r.lower_stat(&s, INTELLIGENCE), Err(Refusal::SpellNeedsIntelligence));
    }
}
