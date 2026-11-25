//! NPCs' reaction toward PCs (Arcanum.exe 0x4c0cc0 / 0x4c0d00 / 0x4c1500 / 0x4c1ac0, change
//! 0x4c0de0, levels 0x4c0fc0).
//!
//! - **Reaction** (0x4c0d00) is the stored value for this PC, else the NPC's
//!   `npc_reaction_base` (50 when the data has none, 0x49b5d9), plus a modifier (0x4c1500):
//!   - unless the NPC has `npc_flags` 0x20000 or `critter_flags` 0x2000800c: the PC's reaction
//!     modifier (stat 14, from Beauty) and [`RACE_REACTION`] by the NPC's and the PC's race;
//!   - the PC's reputations' effects on this NPC ([`ReputationEffects`], `rules/gamerep.mes`).
//!
//!   The result is not clamped.
//! - **Changes** (0x4c0de0) store the unmodified part plus the change, for this PC (the
//!   original keeps ten PCs per NPC and evicts the most neutral; here each PC keeps its own,
//!   saved with it).
//! - **Met** (0x4c0c40) is having a stored value.
//! - **Levels** ([`level`]): 0 above 100 (the friendliest) to 6 at 0 or below.
//!
//! Not built: the PC's effects on reaction (effect type 2, field 0xe4); the charm hold that keeps
//! it at 50 or more (0x459040, flag 0x8000000); the NPC's grudge list (`npc_shit_list_idx`,
//! reaction 0); the follower protection for high Charisma; changes to followers' flags.

use std::collections::HashMap;

use arcanum_formats::mes::MesFile;
use arcanum_formats::object::{Object, Prototypes};
use bevy_ecs::prelude::Resource;

/// The NPC's reaction with no data (0x49b5d9).
pub const BASE_REACTION: i32 = 50;
/// The reputation file.
pub const GAME_REP: &str = "rules/gamerep.mes";

/// Reaction by race (table 0x5b684c): row the NPC's race, column the PC's.
pub const RACE_REACTION: [[i32; 8]; 11] = [
    [0, 0, 5, 5, 0, 10, -5, -5],
    [-5, 10, -10, -5, 0, 5, -10, -15],
    [-10, -20, 0, 0, -10, 10, -15, -10],
    [0, 0, 0, 0, 0, 5, -5, -5],
    [0, 0, 0, 0, 5, 5, -10, 5],
    [0, 0, 20, 15, 0, 10, -10, -10],
    [0, -20, -15, -10, -10, 0, 10, 0],
    [0, -20, -15, -5, 20, 10, 0, 5],
    [-30, -30, 5, -20, -30, -5, -30, -30],
    [-10, -30, -20, -15, -20, -20, 0, -10],
    [-20, -30, -40, -30, -20, -20, 10, 0],
];

/// The level of a reaction (0x4c0fc0): 0 above 100 to 6 at 0 or below.
pub fn level(reaction: i32) -> u32 {
    match reaction {
        r if r <= 0 => 6,
        1..=20 => 5,
        21..=40 => 4,
        41..=60 => 3,
        61..=80 => 2,
        81..=100 => 1,
        _ => 0,
    }
}

/// Each reputation's effects on reaction (`rules/gamerep.mes`, 0x4c1730): `{id}{f1 f2 f3, adj
/// origin faction, ...}`, up to five; an effect applies to NPCs of that origin and faction (0:
/// any).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ReputationEffects(pub HashMap<u32, Vec<(i32, i32, i32)>>);

impl ReputationEffects {
    pub fn parse(mes: &MesFile) -> Self {
        let atoi = |s: &str| arcanum_formats::dialog::atoi(s.as_bytes());
        let effects = mes
            .messages
            .iter()
            .map(|(&id, text)| {
                let list = text
                    .split(',')
                    .skip(1)
                    .take(5)
                    .filter_map(|e| {
                        let parts: Vec<&str> = e.split_whitespace().collect();
                        (parts.len() >= 3).then(|| (atoi(parts[0]), atoi(parts[1]), atoi(parts[2])))
                    })
                    .collect();
                (id, list)
            })
            .collect();
        Self(effects)
    }

    /// The sum of `reputations`' effects on an NPC of `origin` and `faction` (0x4c1ac0).
    pub fn on(&self, reputations: impl IntoIterator<Item = u32>, origin: i32, faction: i32) -> i32 {
        reputations
            .into_iter()
            .filter_map(|r| self.0.get(&r))
            .flatten()
            .filter(|&&(_, o, f)| (o == 0 || o == origin) && (f == 0 || f == faction))
            .map(|&(adj, _, _)| adj)
            .sum()
    }
}

/// What reaction needs of an NPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcReaction {
    pub base: i32,
    pub race: i32,
    pub origin: i32,
    pub faction: i32,
    /// `npc_flags` 0x20000 or `critter_flags` 0x2000800c: looks and race do not count.
    pub only_reputation: bool,
}

impl NpcReaction {
    pub fn of(object: &Object, prototypes: &Prototypes) -> Self {
        let int = |name: &str| prototypes.int(object, name);
        let stats = object
            .ints("critter_stat_base_idx")
            .or_else(|| prototypes.of(object)?.ints("critter_stat_base_idx"));
        Self {
            base: int("npc_reaction_base").unwrap_or(BASE_REACTION),
            race: stats.and_then(|s| s.get(27).copied()).unwrap_or(0),
            origin: int("npc_origin").unwrap_or(0),
            faction: int("npc_faction").unwrap_or(0),
            only_reputation: int("npc_flags").unwrap_or(0) & 0x20000 != 0
                || int("critter_flags").unwrap_or(0) & 0x2000_800c != 0,
        }
    }

    /// The modifier for a PC of `race` with reaction modifier `looks` and these reputations
    /// (0x4c1500).
    pub fn modifier(&self, looks: i32, race: u8, reputations: i32) -> i32 {
        let racial = usize::try_from(self.race)
            .ok()
            .and_then(|r| RACE_REACTION.get(r))
            .and_then(|row| row.get(race as usize))
            .copied()
            .unwrap_or(0);
        if self.only_reputation {
            reputations
        } else {
            looks + racial + reputations
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_run_from_friendly_to_hostile() {
        assert_eq!(
            [
                level(120),
                level(100),
                level(61),
                level(50),
                level(40),
                level(1),
                level(0),
                level(-30)
            ],
            [0, 1, 2, 3, 4, 5, 6, 6]
        );
    }

    #[test]
    fn reputations_count_for_npcs_of_their_origin_and_faction() {
        let mes = MesFile::parse(b"{1000}{0 0 0, +10 0 0}\n{1006}{0 0 0, -20 6 0, -10 0 0}\n{1002}{0 0 0, +10 0 12, -100 0 13}").unwrap();
        let reps = ReputationEffects::parse(&mes);
        assert_eq!(reps.on([1000], 3, 0), 10);
        assert_eq!(reps.on([1006], 6, 0), -30);
        assert_eq!(reps.on([1006], 2, 0), -10);
        assert_eq!(reps.on([1002], 0, 13), -100);
        assert_eq!(reps.on([1000, 1002], 0, 12), 20);
    }

    #[test]
    fn looks_and_race_count_unless_the_npc_ignores_them() {
        let npc = NpcReaction {
            base: 50,
            race: 2,
            origin: 0,
            faction: 0,
            only_reputation: false,
        };
        assert_eq!(
            npc.modifier(25, 1, 0),
            5,
            "Beauty 15 (+25), a dwarf NPC (row 2) to an elf PC (-20)"
        );
        assert_eq!(
            NpcReaction {
                only_reputation: true,
                ..npc
            }
            .modifier(25, 1, 7),
            7
        );
        assert_eq!(
            NpcReaction { race: 40, ..npc }.modifier(0, 0, 0),
            0,
            "no row: nothing"
        );
    }
}
