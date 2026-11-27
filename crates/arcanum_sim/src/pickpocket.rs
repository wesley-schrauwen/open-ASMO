//! Picking pockets: the Pick Pocket skill used on an NPC (Arcanum.exe 0x4c7160 case for skill 5,
//! chance 0x4c62e0 → 0x4c6410, difficulty 0x4c8430, window 0x572240 mode 3, take 0x575180 →
//! 0x4c6fd0).
//!
//! - **Who:** an NPC that is not flagged against it (`critter_flags2` 0x80) and not mechanical
//!   (`critter_flags` 0x20000000); else `mes/skill_ui.mes` 525 (0x57a5e0). The PC walks within
//!   [`crate::USE_RANGE`] first, as for talking.
//! - **The window:** the barter window on the NPC's own inventory, without prices
//!   ([`crate::barter::BarterView::pickpocket`]); the player drags out what to steal. Putting
//!   items in (planting, flag 2) is not built: refused without a word.
//! - **Chance** ([`steal_chance`], 0x4c6410): the thief's Pick Pocket level less the victim's
//!   Perception, from -9 to 9, picks a quarter of table 0x5b7068 (13 to 81); 10 or more over,
//!   100; 10 or more under, 0; at most 95. The difficulty setting's scaling is left out.
//! - **Difficulty** ([`steal_difficulty`], 0x4c8cb9): a worn item by its slot (table 0x5b70e4:
//!   50, 30, 30, 30, 70, 70, then 1000 for the last three), plus its cells across × down × 5,
//!   halved from Expert up.
//! - **Roll** (as Pick Locks'): 1-100 plus the difficulty at or under the chance succeeds; a
//!   failure is critical when a second roll is at or under `max((100 - chance) / 2, 2)`.
//! - **Success:** the item moves to the PC; gold at most 100 coins at a time, ammunition 20.
//!   `skill_ui` 500.
//! - **Failure:** `skill_ui` 501. The victim notices on a critical failure, or when the thief is
//!   untrained and the victim makes a Perception roll (0x45f060; its die was not traced: here
//!   1-20 at or under Perception, builder's call). Then its script runs event 11 (catching a
//!   thief, the PC the triggerer); unless that skips the default, its reaction to the PC drops by
//!   [`CAUGHT_REACTION`] (builder's call: the original's amount, 0x4c0ce0, was not read, and it
//!   turns hostile, 0x4a9650: no combat yet). The window closes either way.
//!
//! Not built: the victim's awareness modifiers (asleep, stunned, unseen: -30 to -50), the thief's
//! blindness and crippled arms, a weapon's to-hit terms, the effect-4 bonus, critical successes,
//! the thief's event 33, the stolen flag (`critter_flags2` 1), planting.

use bevy_ecs::prelude::*;

use crate::{LedgerDelta, LoadedMap, NetId, Outbox, PlayerId, ScriptObject, WorldState};

/// The Pick Pocket skill.
pub const SKILL_PICK_POCKET: usize = 5;
/// `critter_flags2`: cannot be pickpocketed.
pub const OCF2_NO_PICKPOCKET: i32 = 0x80;
/// `critter_flags`: mechanical.
pub const OCF_MECHANICAL: i32 = 0x2000_0000;
/// `mes/skill_ui.mes`: success, failure, and "you cannot pick that pocket".
pub const STEAL_SUCCEEDED: u32 = 500;
pub const STEAL_FAILED: u32 = 501;
pub const CANNOT_STEAL: u32 = 525;
/// The victim's script event when it catches a thief.
pub const EVENT_CATCHING_THIEF: u32 = 11;
/// How much a caught thief loses with the victim (builder's call).
pub const CAUGHT_REACTION: i32 = -10;
/// At most this much gold, or ammunition, is taken at once.
pub const MOST_GOLD: i32 = 100;
pub const MOST_AMMO: i32 = 20;

/// Table 0x5b7068 for differences -9 to 9.
const CHANCES: [i32; 19] = [55, 66, 78, 91, 105, 120, 136, 153, 171, 190, 209, 227, 244, 260, 275, 289, 302, 314, 325];
/// Table 0x5b70e4: what a worn item adds, by slot 1000-1008.
pub const SLOT_DIFFICULTY: [i32; 9] = [50, 30, 30, 30, 70, 70, 1000, 1000, 1000];

/// The chance for a thief of Pick Pocket `level` and `intelligence` against a victim of
/// `perception`: the opposed table, then + 10 at Intelligence 20 (0x4c6369 → 0x4b0ee0 stat 4).
pub fn steal_chance(level: i32, perception: i32, intelligence: i32) -> i32 {
    (opposed_chance(level - perception) + crate::bright_bonus(intelligence)).min(crate::lockpick::MAX_PICK_CHANCE)
}

/// An opposed skill's chance for a level `diff` over the opponent's (0x4c6410): 100 at 10 or
/// more, 0 at −10 or less, else table 0x5b7068 over 4.
pub fn opposed_chance(diff: i32) -> i32 {
    if diff > 9 {
        100
    } else if diff < -9 {
        0
    } else {
        CHANCES[(diff + 9) as usize] / 4
    }
}

/// The difficulty of stealing an item at `location` taking `cells`, for a thief of `training`.
pub fn steal_difficulty(location: i32, cells: (i32, i32), training: u8) -> i32 {
    let slot = usize::try_from(location - crate::FIRST_SLOT).ok().and_then(|i| SLOT_DIFFICULTY.get(i)).copied().unwrap_or(0);
    let size = cells.0 * cells.1 * 5;
    slot + if training >= 2 { size / 2 } else { size }
}

/// Whether NPC `object` can have its pocket picked.
pub fn can_steal_from(object: &arcanum_formats::object::Object, prototypes: &arcanum_formats::object::Prototypes) -> bool {
    object.object_type == arcanum_formats::object::TYPE_NPC
        && prototypes.int(object, "critter_flags2").unwrap_or(0) & OCF2_NO_PICKPOCKET == 0
        && prototypes.int(object, "critter_flags").unwrap_or(0) & OCF_MECHANICAL == 0
}

/// An NPC's Perception (stat 5).
pub fn perception(object: &arcanum_formats::object::Object, prototypes: &arcanum_formats::object::Prototypes) -> i32 {
    let stats = object.ints("critter_stat_base_idx").or_else(|| prototypes.of(object)?.ints("critter_stat_base_idx"));
    stats.and_then(|s| s.get(5).copied()).unwrap_or(0)
}

/// Pick-pocket windows to open, and thieves caught: the player and the NPC.
#[derive(Resource, Debug, Default)]
pub struct PickPocketStarts(pub Vec<(PlayerId, NetId, ScriptObject)>);

#[derive(Resource, Debug, Default)]
pub struct CaughtThieves(pub Vec<(PlayerId, ScriptObject)>);

/// Runs each caught thief's victim's event 11; unless it skips the default, the victim's reaction
/// to the PC drops. The window closes.
pub fn catch_thieves(mut commands: Commands, mut caught: ResMut<CaughtThieves>, mut host: crate::ScriptHost, mut world: ResMut<WorldState>, mut map: ResMut<LoadedMap>, mut outbox: ResMut<Outbox>, pcs: Query<(Entity, &crate::PlayerControlled)>) {
    for (player, victim) in std::mem::take(&mut caught.0) {
        if let Some((entity, _)) = pcs.iter().find(|pc| pc.1.0 == player) {
            commands.entity(entity).remove::<crate::barter::BarterOpen>();
        }
        outbox.0.push((player, LedgerDelta::BarterClosed));
        if host.hook(&mut world, &mut map, &mut outbox, victim, EVENT_CATCHING_THIEF, player) == crate::HookResult::SkipDefault {
            continue;
        }
        let ScriptObject::Mobile { sector, index } = victim else { continue };
        let Some(base) = map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).map(|o| crate::NpcReaction::of(o, &map.prototypes).base) else { continue };
        let key = crate::met_key(victim);
        if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player)) {
            let now = saved.script_state.reactions.get(&key).copied().unwrap_or(base);
            saved.script_state.reactions.insert(key, now + CAUGHT_REACTION);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chance_follows_the_level_against_perception() {
        assert_eq!(steal_chance(8, 8, 8), 47, "even: 190 / 4");
        assert_eq!(steal_chance(0, 9, 8), 13);
        assert_eq!(steal_chance(18, 9, 8), 81);
        assert_eq!(steal_chance(20, 5, 8), 95, "10 over: 100, capped");
        assert_eq!(steal_chance(0, 10, 8), 0);
        assert_eq!(steal_chance(8, 8, 20), 57, "Intelligence 20: + 10");
    }

    #[test]
    fn worn_and_big_items_are_harder() {
        assert_eq!(steal_difficulty(5, (1, 1), 0), 5);
        assert_eq!(steal_difficulty(5, (2, 3), 2), 15, "an expert halves the size");
        assert_eq!(steal_difficulty(1000, (1, 1), 0), 55, "a helmet");
        assert_eq!(steal_difficulty(1008, (1, 1), 3), 1002);
    }
}
