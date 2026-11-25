//! Poison (Arcanum.exe 0x4b1170 / 0x4b1350; stat 0x18): it hurts PCs and NPCs alike over game
//! time and wears off.
//!
//! - **Taking it** (0x4b4390, 0x4b51a4): an attack's poison damage, cut by poison resistance
//!   (0x4b6860), adds to the target's poison level, PC or NPC. NPCs keep it in
//!   [`crate::SectorDelta::npc_poison`].
//! - **Hurting** (time event 6, subtype 0, every [`POISON_MS`] game ms, 0x4b13ab): while the
//!   level is above 0, [`poison_damage`] hit points (1 under 200, 2 under 550, else 3:
//!   0x4b11d1 / 0x4b11e2) as normal damage through 0x4b4390 (0x4b1203). It can kill.
//! - **Wearing off** (subtype 1, every [`POISON_RECOVERY_MS`], 0x4b12bb): the level drops by the
//!   poison recovery rate (stat 0xd; [`arcanum_rules::Derived::poison_recovery`]), to 0.
//! - Builder's calls: one clock for everyone (the original queues each critter's events from
//!   when it was poisoned); the tick's damage is not cut by resistance (0x4b6860 runs on it in
//!   the original; damage resistance in percent of 1-3 rounds to 0 under 50 %); an NPC killed by
//!   poison dies as by a light hit (animation 7), no experience, no script events.

use arcanum_formats::object::{Object, Prototypes};
use bevy_ecs::prelude::*;

/// The poison level's stat (0x18).
pub const STAT_POISON: usize = 0x18;
/// Game ms between poison's hurts (0x4b13ab: 0x3a98).
pub const POISON_MS: u64 = 15_000;
/// Game ms between poison's wearing off (0x4b12bb / 0x4b13fb: 0x1d4c0).
pub const POISON_RECOVERY_MS: u64 = 120_000;

/// Hit points poison level `level` takes each [`POISON_MS`] (0x4b11d1 / 0x4b11e2).
pub fn poison_damage(level: i32) -> i32 {
    match level {
        ..=0 => 0,
        1..200 => 1,
        200..550 => 2,
        _ => 3,
    }
}

/// An NPC's poison recovery rate: stat 0xd, its CN as PCs' ([`arcanum_rules::Derived::poison_recovery`]),
/// at least 1 (builder's call: its effects not counted).
fn npc_recovery(o: &Object, prototypes: &Prototypes) -> i32 {
    crate::combat::npc_stat(o, prototypes, 2).max(1)
}

/// Poison's hurts and wearing off on PCs and NPCs (see the module docs).
#[allow(clippy::too_many_arguments)]
pub fn run_poison(
    clock: Res<crate::GameClock>,
    mut last: Local<Option<(u64, u64)>>,
    rules: Res<crate::SimRules>,
    mut world: ResMut<crate::WorldState>,
    mut sheets: ResMut<crate::ScriptSheets>,
    mut map: ResMut<crate::LoadedMap>,
    mut toggled: ResMut<crate::world::ToggledNpcs>,
    vfs: Option<Res<crate::SimVfs>>,
) {
    let now = crate::magick::game_ms(clock.now);
    let (hurt_from, wear_from) = *last.get_or_insert((now, now));
    let hurts = now.saturating_sub(hurt_from) / POISON_MS;
    let wears = now.saturating_sub(wear_from) / POISON_RECOVERY_MS;
    if hurts == 0 && wears == 0 {
        return;
    }
    *last = Some((hurt_from + hurts * POISON_MS, wear_from + wears * POISON_RECOVERY_MS));
    if let Some(save) = world.0.as_mut() {
        for (&player, saved) in save.players.iter_mut().filter(|(_, s)| s.blueprint.sheet.poison > 0 && !s.blueprint.sheet.dead) {
            let sheet = &mut saved.blueprint.sheet;
            let derived = rules.0.derived(sheet);
            sheet.hp_damage = (sheet.hp_damage + poison_damage(sheet.poison) * hurts as i32).min(derived.hit_points);
            sheet.poison = (sheet.poison - derived.poison_recovery.max(0) * wears as i32).max(0);
            sheets.0.insert(player);
        }
    }
    // NPCs poisoned in this game (an NPC's own stat 0x18 from the data is not counted).
    let poisoned: Vec<(arcanum_formats::map::SectorId, u32, i32)> = map.changes.iter().flat_map(|(&s, c)| c.npc_poison.iter().filter(|(_, p)| **p > 0).map(move |(&i, &p)| (s, i, p))).collect();
    let guard = vfs.as_ref().and_then(|v| v.0.read().ok());
    for (sector, index, level) in poisoned {
        if map.is_dead(sector, index) {
            continue;
        }
        let Some(rate) = map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).map(|o| npc_recovery(o, &map.prototypes)) else { continue };
        let damage = poison_damage(level) * hurts as i32;
        if damage > 0 {
            let left = map.npc_hp(sector, index).map_or(0, |h| h.0);
            map.hurt_npc(sector, index, damage);
            if left - damage < 1 {
                map.kill_npc(guard.as_deref(), sector, index, crate::ANIM_DEATH);
                toggled.0.push((sector, index));
            }
        }
        map.set_npc_poison(sector, index, level - rate * wears as i32);
        crate::ground::save_change(&mut world, &map, sector);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poison_hurts_by_its_level() {
        assert_eq!([0, 1, 199, 200, 549, 550, 1000].map(poison_damage), [0, 1, 1, 2, 2, 3, 3]);
    }
}
