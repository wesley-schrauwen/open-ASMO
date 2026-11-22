//! Skill, tech, spell and schematic tables from the game's text files.
//!
//! - `mes/skill.mes`: basic skills 0-11, tech skills 12-15, training ranks 17-19 (Apprentice,
//!   Expert, Master), descriptions 20-35.
//! - `mes/tech.mes`: disciplines 0-7, degrees 8-15 (Layman .. Doctorate), discipline
//!   descriptions 16-23, degree descriptions 24 + 8 x discipline + degree.
//! - `mes/spell.mes`: spells 0-79 (five per college, in college order), colleges 500-515,
//!   descriptions 700 + spell.
//! - `rules/schematic.mes`: blocks at multiples of 10: name, text (`mes/schematic_text.mes`),
//!   drawing (`art/interface/interface.mes`), item 1, item 2, result (each up to three prototype
//!   numbers for sized items), amount. 2000-3999 are learned with a degree (2000 + 200 x
//!   discipline + 10 x (degree - 1)), 4000-5999 found (4000 + 200 x discipline).

use arcanum_formats::mes::MesFile;

pub const BASIC_SKILLS: usize = 12;
pub const TECH_SKILLS: usize = 4;
pub const SKILLS: usize = BASIC_SKILLS + TECH_SKILLS;
pub const DISCIPLINES: usize = 8;
pub const DEGREES: usize = 8;
pub const SPELLS: usize = 80;
pub const COLLEGES: usize = 16;
pub const SPELLS_PER_COLLEGE: usize = 5;

fn text(mes: &MesFile, id: usize) -> String {
    mes.get(id as u32).unwrap_or("").to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillTable {
    /// Basic skills 0-11, then tech skills 12-15.
    pub skills: Vec<Skill>,
    /// Apprentice, Expert, Master.
    pub ranks: [String; 3],
}

impl SkillTable {
    pub fn from_mes(mes: &MesFile) -> Self {
        Self {
            skills: (0..SKILLS).map(|i| Skill { name: text(mes, i), description: text(mes, 20 + i) }).collect(),
            ranks: std::array::from_fn(|i| text(mes, 17 + i)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discipline {
    pub name: String,
    pub description: String,
    /// What each degree's schematic makes (index 0 = Layman, "the lack of knowledge").
    pub degree_descriptions: [String; DEGREES],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TechTable {
    pub disciplines: Vec<Discipline>,
    /// Layman .. Doctorate.
    pub degrees: [String; DEGREES],
}

impl TechTable {
    pub fn from_mes(mes: &MesFile) -> Self {
        Self {
            disciplines: (0..DISCIPLINES)
                .map(|d| Discipline {
                    name: text(mes, d),
                    description: text(mes, 16 + d),
                    degree_descriptions: std::array::from_fn(|k| text(mes, 24 + DEGREES * d + k)),
                })
                .collect(),
            degrees: std::array::from_fn(|k| text(mes, 8 + k)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Spell {
    pub name: String,
    pub description: String,
    /// `spell / 5`.
    pub college: usize,
    /// Fatigue spent casting it (`rules/spelllist.mes` line `1001 + 50 * spell`, `Cost:`).
    pub cost: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpellTable {
    pub spells: Vec<Spell>,
    pub colleges: Vec<String>,
}

impl SpellTable {
    pub fn from_mes(mes: &MesFile) -> Self {
        Self {
            spells: (0..SPELLS)
                .map(|i| Spell { name: text(mes, i), description: text(mes, 700 + i), college: i / SPELLS_PER_COLLEGE, cost: 0 })
                .collect(),
            colleges: (0..COLLEGES).map(|c| text(mes, 500 + c)).collect(),
        }
    }

    /// Reads each spell's cost from `rules/spelllist.mes` (the `Cost:` option of line
    /// `1001 + 50 * spell`; spells with none keep 0).
    pub fn read_costs(&mut self, list: &MesFile) {
        for (i, spell) in self.spells.iter_mut().enumerate() {
            let options = list.get(1001 + 50 * i as u32).unwrap_or("");
            spell.cost = spell_option(options, "Cost").unwrap_or(0);
        }
    }
}

/// The number after `name:` in a spell's option list, e.g. `Cost: 10, Info: aggressive`.
fn spell_option(options: &str, name: &str) -> Option<i32> {
    options.split(',').find_map(|o| {
        let (key, value) = o.split_once(':')?;
        (key.trim().eq_ignore_ascii_case(name)).then(|| value.trim().parse().ok()).flatten()
    })
}

/// How a schematic is obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchematicSource {
    /// Granted by a discipline degree (1 = Novice .. 7 = Doctorate).
    Learned { discipline: usize, degree: usize },
    /// Found in the world; ids from 6000 have no discipline range.
    Found { discipline: Option<usize> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schematic {
    /// The block's first line number.
    pub id: u32,
    /// `mes/schematic_text.mes` ids.
    pub name: u32,
    pub text: u32,
    /// `art/interface/interface.mes` id.
    pub drawing: u32,
    /// Prototype numbers for each ingredient (up to three for sized items).
    pub ingredients: [Vec<u32>; 2],
    pub result: Vec<u32>,
    pub amount: u32,
    pub source: SchematicSource,
}

fn numbers(mes: &MesFile, id: u32) -> Vec<u32> {
    mes.get(id).unwrap_or("").split_whitespace().filter_map(|t| t.parse().ok()).collect()
}

/// Every schematic in `rules/schematic.mes` (blocks from 2000 on).
pub fn parse_schematics(mes: &MesFile) -> Vec<Schematic> {
    let first = |id: u32| numbers(mes, id).first().copied();
    (2000..10_000u32)
        .step_by(10)
        .filter(|&id| (0..7).any(|k| mes.get(id + k).is_some()))
        .map(|id| {
            let source = match id {
                2000..=3999 => {
                    let off = (id - 2000) as usize;
                    SchematicSource::Learned { discipline: off / 200, degree: off % 200 / 10 + 1 }
                }
                4000..=5999 => SchematicSource::Found { discipline: Some((id as usize - 4000) / 200) },
                _ => SchematicSource::Found { discipline: None },
            };
            Schematic {
                id,
                name: first(id).unwrap_or(0),
                text: first(id + 1).unwrap_or(0),
                drawing: first(id + 2).unwrap_or(0),
                ingredients: [numbers(mes, id + 3), numbers(mes, id + 4)],
                result: numbers(mes, id + 5),
                amount: first(id + 6).unwrap_or(0),
                source,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mes(text: &str) -> MesFile {
        MesFile::parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn spell_costs_come_from_the_spell_list() {
        let mut table = SpellTable::from_mes(&mes("{0}{Disarm}{1}{Unlocking Cantrip}"));
        table.read_costs(&mes("{1001}{Cost: 5, Resist: (stat_dexterity @ 0), Info: aggressive}{1051}{Cost: 10}{1201}{Cost: 50, ChargeBeginCost: 0}"));
        assert_eq!((table.spells[0].cost, table.spells[1].cost, table.spells[4].cost, table.spells[2].cost), (5, 10, 50, 0));
    }

    #[test]
    fn schematics_know_where_they_come_from() {
        let list = parse_schematics(&mes(
            "{2000}{10}{2001}{11}{2002}{532}{2003}{10061}{2004}{10062}{2005}{10059}{2006}{5}\
             {4210}{1}{4211}{2}{4212}{3}{4213}{1 2 3}{4214}{4}{4215}{5}{4216}{1}",
        ));
        assert_eq!(list.len(), 2);
        let a = &list[0];
        assert_eq!((a.id, a.name, a.text, a.drawing, a.amount), (2000, 10, 11, 532, 5));
        assert_eq!(a.source, SchematicSource::Learned { discipline: 0, degree: 1 });
        assert_eq!((a.ingredients.clone(), a.result.clone()), ([vec![10061], vec![10062]], vec![10059]));
        let b = &list[1];
        assert_eq!(b.source, SchematicSource::Found { discipline: Some(1) });
        assert_eq!(b.ingredients[0], [1, 2, 3]);
    }

    #[test]
    fn learned_schematic_degrees_and_late_found_ones() {
        let list = parse_schematics(&mes("{3460}{1}{3466}{1}{6010}{2}{6016}{1}"));
        assert_eq!(list[0].source, SchematicSource::Learned { discipline: 7, degree: 7 });
        assert_eq!(list[1].source, SchematicSource::Found { discipline: None });
    }

    #[test]
    fn bad_item_numbers_are_skipped() {
        let list = parse_schematics(&mes("{2000}{10}{2003}{12 x 13}{2006}{1}"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].ingredients[0], [12, 13]);
    }

    #[test]
    fn spells_belong_to_colleges_in_order() {
        let t = SpellTable::from_mes(&mes("{0}{Disarm}{7}{Read Aura}{500}{Conveyance}{501}{Divination}{707}{Reveals auras.}"));
        assert_eq!(t.spells.len(), SPELLS);
        assert_eq!((t.spells[7].name.as_str(), t.spells[7].college, t.spells[7].description.as_str()), ("Read Aura", 1, "Reveals auras."));
        assert_eq!(&t.colleges[..2], ["Conveyance", "Divination"]);
    }

    #[test]
    fn skills_and_ranks_are_read() {
        let t = SkillTable::from_mes(&mes("{0}{Bow}{12}{Repair}{17}{Apprentice}{18}{Expert}{19}{Master}{20}{Bows. (DX)}"));
        assert_eq!(t.skills.len(), SKILLS);
        assert_eq!((t.skills[0].name.as_str(), t.skills[0].description.as_str()), ("Bow", "Bows. (DX)"));
        assert_eq!(t.skills[12].name, "Repair");
        assert_eq!(t.ranks, ["Apprentice", "Expert", "Master"]);
    }

    #[test]
    fn disciplines_and_degrees_are_read() {
        let t = TechTable::from_mes(&mes("{0}{Herbology}{8}{Layman}{15}{Doctorate}{16}{Herbs.}{25}{Heal Lite text}"));
        assert_eq!(t.disciplines.len(), DISCIPLINES);
        assert_eq!((t.disciplines[0].name.as_str(), t.disciplines[0].description.as_str()), ("Herbology", "Herbs."));
        assert_eq!(t.disciplines[0].degree_descriptions[1], "Heal Lite text");
        assert_eq!((t.degrees[0].as_str(), t.degrees[7].as_str()), ("Layman", "Doctorate"));
    }
}
