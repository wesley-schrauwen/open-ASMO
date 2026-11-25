//! Casting spells (Arcanum.exe 0x57bc70 / 0x57c110 / 0x4564e0 / 0x4508a0 / 0x450420): for now
//! the Unlocking Cantrip on a door or container.
//!
//! - **Who may cast** (0x57bc70, 0x4564e0): a PC who knows the spell (its college's count,
//!   [`arcanum_rules::CharacterSheet::spells`], is past the spell's place in it) with an
//!   Intelligence of at least [`MIN_INTELLIGENCE`] (0x4502e0 returns 5 for every spell; the
//!   list's 6-18 by level is a comment only).
//! - **Cost** (0x4508a0 → 0x450420): the spell's `Cost:` in fatigue, refused with "Not enough
//!   Energy." (`mes/spell.mes` 600) when it would take the PC below [`LOWEST_FATIGUE`]. Spent as
//!   fatigue damage ([`arcanum_rules::CharacterSheet::fatigue_damage`]), saved and shown on the
//!   vial.
//! - **Range**: the list's default is "max visible"; [`CAST_RANGE`] stands in for it.
//! - **The Unlocking Cantrip** (spell 1, `rules/spelllist.mes` 1050-1057): any portal or
//!   container, locked or not; clears the locked bit (`OPF_LOCKED` / `OCOF_LOCKED`), leaves a
//!   jam and does not open it.
//! - **Recovery** (0x45d5a0 schedules a time event 80 000 game ms after fatigue is spent; its
//!   handler 0x45e610 heals): every [`RECOVERY_MS`] the fatigue damage drops by the heal rate
//!   (stat 12, `(CN + 1) / 3`), down to 0. (0x45e910's three times the rate is resting,
//!   [`crate::rest`].)
//! - **Unconscious** (0x45d800): at fatigue 0 or below the PC casts nothing
//!   ([`crate::unconscious`]).
//!
//! Not built: the cast animation and its eye candy and sounds (`SpellEyeCandy.mes`: cast art
//! 10, sound 9020; destination art 16, sound 9025); the charge waiting for the animation; mana
//! stored in worn items, drawn first; line of sight ("You cannot see the target.", 604); the
//! tech-aptitude roll (read as dead code); other spells.

use arcanum_rules::{CharacterSheet, SPELLS_PER_COLLEGE};
use bevy_ecs::prelude::{Commands, Component, Entity, Query, Res, ResMut, Resource};

use crate::{LedgerDelta, LoadedMap, LockTarget, NetId, Outbox, PlayerControlled, PlayerId, Position, Session, Sheet, SimRules, WorldState, pc_status};

/// The Unlocking Cantrip.
pub const SPELL_UNLOCKING_CANTRIP: usize = 1;
/// Intelligence needed for any spell (0x4502e0).
pub const MIN_INTELLIGENCE: i32 = 5;
/// The lowest fatigue a cast may leave (0x450420 refuses below -14).
pub const LOWEST_FATIGUE: i32 = -14;
/// How far a spell reaches, in tiles: the list's "max visible" (builder's call, about the
/// 800x400 view's half width).
pub const CAST_RANGE: u32 = 15;
/// `mes/spell.mes`: "Not enough Energy."
pub const NOT_ENOUGH_ENERGY: u32 = 600;
/// Game milliseconds between fatigue recoveries (0x45e820's 80 000).
pub const RECOVERY_MS: u64 = 80_000;

/// Fatigue damage after `steps` recoveries at `heal_rate` (0x45e610: the rate each).
pub fn recovered(damage: i32, heal_rate: i32, steps: u64) -> i32 {
    (damage as i64 - heal_rate as i64 * steps as i64).max(0) as i32
}

/// When a PC's next fatigue recovery is due, in game milliseconds.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FatigueRecovery {
    pub due: u64,
}

fn game_ms(time: crate::GameTime) -> u64 {
    time.days as u64 * crate::DAY_MS as u64 + time.ms as u64
}

/// Heals the PCs' spent fatigue as game time passes: see the module docs.
#[allow(clippy::too_many_arguments)]
pub fn recover_fatigue(
    mut commands: Commands,
    clock: Res<crate::GameClock>,
    mut world: ResMut<WorldState>,
    session: Res<Session>,
    rules: Res<SimRules>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<(Entity, &NetId, &PlayerControlled, &mut Sheet, Option<&FatigueRecovery>)>,
) {
    let now = game_ms(clock.now);
    for (entity, &net_id, player, mut sheet, recovery) in &mut pcs {
        if sheet.0.fatigue_damage <= 0 {
            if recovery.is_some() {
                commands.entity(entity).remove::<FatigueRecovery>();
            }
            continue;
        }
        let Some(&FatigueRecovery { due }) = recovery else {
            commands.entity(entity).insert(FatigueRecovery { due: now + RECOVERY_MS });
            continue;
        };
        if now < due {
            continue;
        }
        let steps = 1 + (now - due) / RECOVERY_MS;
        sheet.0.fatigue_damage = recovered(sheet.0.fatigue_damage, rules.0.derived(&sheet.0).heal_rate, steps);
        commands.entity(entity).insert(FatigueRecovery { due: due + steps * RECOVERY_MS });
        if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player.0)) {
            saved.blueprint.sheet = sheet.0.clone();
        }
        crate::ground::broadcast(&session, &mut outbox, LedgerDelta::PcStatus { net_id, status: pc_status(&rules.0, &sheet.0) });
    }
}

/// Whether `sheet` knows `spell`: spells are known in order within a college.
pub fn knows_spell(sheet: &CharacterSheet, spell: usize) -> bool {
    sheet.spells.get(spell / SPELLS_PER_COLLEGE).is_some_and(|&known| known > (spell % SPELLS_PER_COLLEGE) as i32)
}

/// Whether fatigue `current` pays `cost` (0x450420).
pub fn affords(current: i32, cost: i32) -> bool {
    current - cost >= LOWEST_FATIGUE
}

/// Why a cast did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastRefusal {
    /// Not known, too dim, unconscious, no such target or out of range: nothing is said.
    Silent,
    /// "Not enough Energy."
    NoEnergy,
}

/// Checks a cast of `spell` by `sheet` with fatigue `current`; `Ok` is the cost.
pub fn cast_check(rules: &arcanum_rules::RulesData, sheet: &CharacterSheet, current: i32, spell: usize) -> Result<i32, CastRefusal> {
    if !knows_spell(sheet, spell) || rules.stats(sheet)[4] < MIN_INTELLIGENCE || current < 1 {
        return Err(CastRefusal::Silent);
    }
    let cost = rules.spells.spells.get(spell).map_or(0, |s| s.cost);
    if !affords(current, cost) {
        return Err(CastRefusal::NoEnergy);
    }
    Ok(cost)
}

/// Casts taken from the inbox by `handle_intents`: the player, the spell and its target.
#[derive(Resource, Debug, Default)]
pub struct CastRequests(pub Vec<(PlayerId, usize, LockTarget)>);

/// Casts the requested spells from where the PCs stand.
#[allow(clippy::too_many_arguments)]
pub fn cast_spells(
    mut requests: ResMut<CastRequests>,
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    session: Res<Session>,
    rules: Res<SimRules>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<(&NetId, &PlayerControlled, &Position, &mut Sheet)>,
) {
    for (player, spell, target) in requests.0.drain(..) {
        let Some((&net_id, _, position, mut sheet)) = pcs.iter_mut().find(|pc| pc.1.0 == player) else { continue };
        if spell != SPELL_UNLOCKING_CANTRIP {
            continue;
        }
        let Some(location) = map.lock_location(target) else { continue };
        if !crate::within_radius(position.0, location, CAST_RANGE) {
            continue;
        }
        let current = pc_status(&rules.0, &sheet.0).fatigue.current;
        let cost = match cast_check(&rules.0, &sheet.0, current, spell) {
            Ok(cost) => cost,
            Err(CastRefusal::NoEnergy) => {
                outbox.0.push((player, LedgerDelta::SpellResult { message: NOT_ENOUGH_ENERGY }));
                continue;
            }
            Err(CastRefusal::Silent) => continue,
        };
        let Some(flags) = map.lock_flags(target) else { continue };
        sheet.0.fatigue_damage += cost;
        if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player)) {
            saved.blueprint.sheet = sheet.0.clone();
        }
        let status = pc_status(&rules.0, &sheet.0);
        crate::ground::broadcast(&session, &mut outbox, LedgerDelta::PcStatus { net_id, status });
        let changed = flags & !crate::OPF_LOCKED;
        if changed != flags {
            map.set_lock_flags(target, changed);
            let sector = match target {
                LockTarget::Door(id) => id.sector,
                LockTarget::Container(id) => id.sector(),
            };
            crate::ground::save_change(&mut world, &map, sector);
            crate::ground::broadcast(&session, &mut outbox, LedgerDelta::LockChanged { target, flags: changed });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fatigue_recovers_the_heal_rate_a_step() {
        assert_eq!(recovered(20, 3, 1), 17);
        assert_eq!(recovered(20, 3, 7), 0, "not below 0");
        assert_eq!(recovered(5, 0, 4), 5, "no heal rate");
    }

    #[test]
    fn spells_are_known_in_order_within_a_college() {
        let mut sheet = CharacterSheet::default();
        assert!(!knows_spell(&sheet, SPELL_UNLOCKING_CANTRIP));
        sheet.spells[0] = 1;
        assert!(knows_spell(&sheet, 0) && !knows_spell(&sheet, 1));
        sheet.spells[0] = 2;
        assert!(knows_spell(&sheet, SPELL_UNLOCKING_CANTRIP));
        sheet.spells[3] = 5;
        assert!(knows_spell(&sheet, 19) && !knows_spell(&sheet, 20));
        assert!(!knows_spell(&sheet, 500));
    }

    #[test]
    fn a_cast_may_leave_fatigue_down_to_minus_14() {
        assert!(affords(10, 10));
        assert!(affords(0, 14));
        assert!(!affords(0, 15));
        let mut rules = arcanum_rules::RulesData::default();
        rules.spells.spells = (0..arcanum_rules::SPELLS).map(|i| arcanum_rules::Spell { name: String::new(), description: String::new(), college: i / 5, cost: 10 }).collect();
        let mut sheet = CharacterSheet::default();
        assert_eq!(cast_check(&rules, &sheet, 30, 1), Err(CastRefusal::Silent), "not known");
        sheet.spells[0] = 2;
        assert_eq!(cast_check(&rules, &sheet, 30, 1), Ok(10));
        assert_eq!(cast_check(&rules, &sheet, 3, 1), Ok(10), "down to -7");
        assert_eq!(cast_check(&rules, &sheet, 0, 1), Err(CastRefusal::Silent), "unconscious");
    }
}
