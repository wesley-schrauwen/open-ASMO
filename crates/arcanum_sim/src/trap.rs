//! Traps: spotting them and disarming them (Arcanum.exe 0x4bbe40, 0x4bc7b0, 0x4bc220; the trap
//! itself is sprung by the script runner, [`crate::script`]).
//!
//! - **A trap** is a trap script ([`crate::is_trap`]) attached to an object's use (event 1).
//! - **Spotting** (0x4bbe40, Spot Trap, skill 7, 0x4c7160 case 7): when a PC uses an object
//!   whose trap is not spotted. The chance is the skill's ([`spot_chance`], 0x4c62e0: 4 a level);
//!   a Master who fails rolls again; a success below Expert on a magical trap fails. Spotted,
//!   the object gets flag [`OF_TRAP_SPOTTED`] (0x4bc090) and the PC is told `mes/trap.mes` 0.
//! - **Disarming** (Disarm Traps, skill 15, 0x4c7160 case 0xf → 0x4bc7b0): only a spotted trap.
//!   Its chance is [`crate::tech_chance`]'s (4 a level + 20, none at level 0).
//!   The roll is the lock pick's ([`crate::roll_pick`]; a door's or container's trap has no
//!   difficulty, 0x4c8430 adds `trap_difficulty` only for trap objects); a Master who fails
//!   without a critical failure rolls again. Success removes the trap (0x4bc220: the
//!   attachment's script number becomes 0); a critical failure springs it on the PC (0x441980
//!   event 1). `mes/skill_ui.mes` 560 / 561 tell the result.
//!
//! - **Spotting while walking** (0x427110, each step): a PC about to step onto a tile with a
//!   trap object (type 17, list mask 0x20000) it has not spotted rolls Spot Trap, once a tile;
//!   spotting it stops the walk before the step. Doors and containers are not checked this way.
//!   NPCs know their traps (0x4bbfe0); the original's followers roll too (not built).
//!
//! Not built: stepping onto a trap object (what springs it was not found); the recovered trap
//! parts on a critical success for an Expert or Master (0x4bc9c0, table
//! 0x5b5f80; the critical success roll 0x4c81a0 is not decoded); setting traps (0x4bc480);
//! the eye candy (`rules/TrapEyeCandy.mes` N × 10 + 2 on a spotted trap); scenery and trap
//! objects as targets (doors and containers only).

use bevy_ecs::prelude::*;

use crate::{LedgerDelta, LoadedMap, LockTarget, Outbox, PlayerControlled, PlayerId, Position, Sheet, SimRules, WorldState};

/// `flags`: the object's trap has been spotted (0x4bc090).
pub const OF_TRAP_SPOTTED: i32 = 0x200_0000;
/// `flags`: a trap a PC set (0x4bc480): PCs know it.
pub const OF_TRAP_PLACED_BY_PC: i32 = 0x100_0000;
/// The Spot Trap and Disarm Traps skills (`mes/skill.mes` order).
pub const SKILL_SPOT_TRAP: usize = 7;
pub const SKILL_DISARM_TRAPS: usize = 15;
/// The trap texts.
pub const TRAP_MES: &str = "mes/trap.mes";
/// `mes/skill_ui.mes`: disarmed, failed.
pub const DISARM_SUCCEEDED: u32 = 560;
pub const DISARM_FAILED: u32 = 561;
const EXPERT: u8 = 2;
const MASTER: u8 = 3;

/// Spot Trap's chance (0x4c62e0, a basic skill: 0x4c71b9 sends skills under 12 there): 4 a
/// level (0x4c6350, table 0x4c63dc entry 7), + 10 at Intelligence 20, at most 95 (flag 0x884
/// holds 0x800).
pub fn spot_chance(level: i32, intelligence: i32) -> i32 {
    (level.max(0) * 4 + crate::bright_bonus(intelligence)).clamp(0, crate::MAX_PICK_CHANCE)
}

/// A Spot Trap roll (0x4bbe40 → 0x4c7160 case 7).
/// `light`: the luminance where the trap is (the dark adds to the roll below Apprentice,
/// [`crate::light::skill_dark`]).
pub fn roll_spot(rules: &arcanum_rules::RulesData, sheet: &arcanum_rules::CharacterSheet, magical: bool, light: Option<i32>, rng: &mut crate::HostRng) -> bool {
    let chance = spot_chance(rules.skill_level(sheet, SKILL_SPOT_TRAP), rules.stats(sheet)[4]);
    let training = sheet.training[SKILL_SPOT_TRAP];
    let dark = crate::light::skill_dark(training, 1, light);
    let mut spotted = rng.range(1, 100) + dark <= chance;
    if !spotted && training >= MASTER {
        spotted = rng.range(1, 100) <= chance;
    }
    spotted && !(magical && training < EXPERT)
}

/// A Disarm Traps roll (0x4c7160 case 0xf): success, failure, or a critical failure
/// ([`crate::PickOutcome::Jammed`]).
/// `light`: the luminance where the trap is ([`crate::light::skill_dark`], below Apprentice).
pub fn roll_disarm(chance: i32, training: u8, light: Option<i32>, rng: &mut crate::HostRng) -> crate::PickOutcome {
    let dark = crate::light::skill_dark(training, 1, light);
    let outcome = crate::roll_pick(chance, dark, rng);
    if outcome == crate::PickOutcome::Failure && training >= MASTER { crate::roll_pick(chance, dark, rng) } else { outcome }
}

/// The tile a walking PC last rolled for.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpotChecked(pub arcanum_formats::map::TileLoc);

/// The trap objects (type 17) on `at` whose trap is not spotted: sector, index, magical.
fn unspotted_traps(map: &LoadedMap, host: &mut crate::script_host::ScriptHost, at: arcanum_formats::map::TileLoc) -> Vec<(arcanum_formats::map::SectorId, u32, bool)> {
    let sector = at.sector();
    let candidates: Vec<(u32, u32)> = map
        .mobiles
        .get(&sector)
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, o)| o.object_type == arcanum_formats::object::TYPE_TRAP && crate::object_location(o) == Some(at))
        .filter(|(_, o)| map.prototypes.int(o, "flags").unwrap_or(0) & (OF_TRAP_SPOTTED | OF_TRAP_PLACED_BY_PC | crate::OF_OFF | crate::OF_DESTROYED) == 0)
        .map(|(i, _)| (i as u32, map.attachment(crate::ScriptObject::Mobile { sector, index: i as u32 }, crate::EVENT_USE).number))
        .collect();
    candidates
        .into_iter()
        .filter_map(|(index, number)| {
            let flags = host.script_flags(number)?;
            crate::is_trap(number, &arcanum_formats::script::Script { description: String::new(), flags, lines: vec![] }).then_some((sector, index, flags & crate::TRAP_MAGICAL != 0))
        })
        .collect()
}

type SpotterQuery<'a> = (Entity, &'a PlayerControlled, &'a mut crate::Walking, &'a Sheet, Option<&'a SpotChecked>);

/// A walking PC rolls Spot Trap for each trap object on the tile its next step enters, once a
/// tile; spotting one marks it, tells the player and stops the walk where the PC stands.
#[allow(clippy::too_many_arguments)]
pub fn spot_traps_walking(
    mut commands: Commands,
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    mut host: crate::script_host::ScriptHost,
    rules: Res<SimRules>,
    session: Res<crate::Session>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<SpotterQuery>,
    idle: Query<Entity, (With<SpotChecked>, Without<crate::Walking>)>,
    clock: Res<crate::GameClock>,
) {
    // A PC that stopped rolls again on its next walk.
    for entity in &idle {
        commands.entity(entity).remove::<SpotChecked>();
    }
    for (entity, controlled, mut walking, sheet, checked) in &mut pcs {
        let Some(&next) = walking.steps.front() else { continue };
        if checked.is_some_and(|c| c.0 == next) {
            continue;
        }
        commands.entity(entity).insert(SpotChecked(next));
        let mut spotted = false;
        for (sector, index, magical) in unspotted_traps(&map, &mut host, next) {
            if !roll_spot(&rules.0, &sheet.0, magical, map.luminance_at(next, clock.now), host.rng()) {
                continue;
            }
            spotted = true;
            let flags = map.object_flags(sector, index, true).unwrap_or(0) | OF_TRAP_SPOTTED;
            map.set_object_flags(sector, index, true, flags);
            crate::ground::save_change(&mut world, &map, sector);
            crate::ground::broadcast(&session, &mut outbox, LedgerDelta::ObjectFlags { sector, index, mobile: true, flags });
        }
        if spotted {
            walking.steps.clear();
            let text = host.trap_line(crate::TRAP_LINE_SPOTTED);
            if let Some(text) = text {
                outbox.0.push((controlled.0, LedgerDelta::ScriptMessage { text }));
            }
        }
    }
}

/// Players' requests to disarm a trap: the target and the move sent with it.
#[derive(Resource, Debug, Default)]
pub struct DisarmRequests(pub Vec<(PlayerId, LockTarget, u32)>);

/// A PC's disarm waiting for it to walk in range.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingDisarm {
    pub target: LockTarget,
    pub seq: u32,
}

/// The script object a door or container is.
pub fn target_object(target: LockTarget) -> Option<crate::ScriptObject> {
    match target {
        LockTarget::Door(id) => Some(crate::ScriptObject::Map(id)),
        LockTarget::Container(id) => crate::ScriptObject::ground(id),
    }
}

type DisarmerQuery<'a> = (Entity, &'a PlayerControlled, &'a Position, Option<&'a crate::Walking>, Option<&'a PendingDisarm>, &'a Sheet);

/// Takes disarm requests (each becomes the PC's [`PendingDisarm`]), then disarms the traps whose
/// PC is within [`crate::USE_RANGE`]; a request whose PC stopped out of range is dropped.
#[allow(clippy::too_many_arguments)]
pub fn disarm_traps(
    mut commands: Commands,
    mut requests: ResMut<DisarmRequests>,
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    mut host: crate::script_host::ScriptHost,
    rules: Res<SimRules>,
    mut outbox: ResMut<Outbox>,
    pcs: Query<DisarmerQuery>,
    clock: Res<crate::GameClock>,
) {
    let mut fresh = std::collections::HashMap::new();
    for (player, target, seq) in requests.0.drain(..) {
        if let Some((entity, ..)) = pcs.iter().find(|pc| pc.1.0 == player) {
            fresh.insert(entity, PendingDisarm { target, seq });
        }
    }
    for (entity, controlled, position, walking, pending, sheet) in &pcs {
        let Some(want) = fresh.get(&entity).copied().or(pending.copied()) else { continue };
        let (Some(location), Some(object)) = (map.lock_location(want.target), target_object(want.target)) else {
            commands.entity(entity).remove::<PendingDisarm>();
            continue;
        };
        if !crate::within_radius(position.0, location, crate::USE_RANGE) {
            if walking.is_some() {
                commands.entity(entity).insert(want);
            } else {
                commands.entity(entity).remove::<PendingDisarm>();
            }
            continue;
        }
        commands.entity(entity).remove::<PendingDisarm>();
        let player = controlled.0;
        let attachment = map.attachment(object, crate::EVENT_USE);
        let place = match object {
            crate::ScriptObject::Map(id) => Some((id.sector, id.index, false)),
            crate::ScriptObject::Mobile { sector, index } => Some((sector, index, true)),
            _ => None,
        };
        let spotted = place.and_then(|(s, i, m)| map.object_flags(s, i, m)).is_some_and(|f| f & OF_TRAP_SPOTTED != 0);
        if !spotted || !host.is_trap(attachment.number) {
            outbox.0.push((player, LedgerDelta::SkillResult { message: DISARM_FAILED }));
            continue;
        }
        let sheet = &sheet.0;
        let chance = crate::tech_chance(crate::TECH_DISARM_TRAPS, rules.0.skill_level(sheet, SKILL_DISARM_TRAPS), rules.0.stats(sheet)[4]);
        match roll_disarm(chance, sheet.training[SKILL_DISARM_TRAPS], map.luminance_at(location, clock.now), host.rng()) {
            crate::PickOutcome::Success => {
                map.set_attachment(object, crate::EVENT_USE, crate::Attachment { number: 0, ..attachment });
                if let Some((sector, ..)) = place {
                    crate::ground::save_change(&mut world, &map, sector);
                }
                outbox.0.push((player, LedgerDelta::SkillResult { message: DISARM_SUCCEEDED }));
            }
            crate::PickOutcome::Failure => outbox.0.push((player, LedgerDelta::SkillResult { message: DISARM_FAILED })),
            crate::PickOutcome::Jammed => {
                outbox.0.push((player, LedgerDelta::SkillResult { message: DISARM_FAILED }));
                host.hook(&mut world, &mut map, &mut outbox, object, crate::EVENT_USE, player);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(level_points: i32, training: u8, skill: usize) -> arcanum_rules::CharacterSheet {
        let mut s = arcanum_rules::RulesData::default().new_sheet();
        s.skills[skill] = level_points;
        s.training[skill] = training;
        s.base = [20, 20, 20, 20, 8, 20, 20, 20];
        s
    }

    #[test]
    fn spotting_follows_the_skill_and_magic_needs_an_expert() {
        let rules = arcanum_rules::RulesData::default();
        let mut rng = crate::HostRng::default();
        let blind = sheet(0, 0, SKILL_SPOT_TRAP);
        assert!((0..200).all(|_| !roll_spot(&rules, &blind, false, None, &mut rng)), "no skill: never");
        let keen = sheet(5, 0, SKILL_SPOT_TRAP);
        let seen = (0..1000).filter(|_| roll_spot(&rules, &keen, false, None, &mut rng)).count();
        assert!((740..860).contains(&seen), "level 20: 4 a level, 80 %, {seen}");
        assert_eq!((spot_chance(10, 8), spot_chance(10, 20), spot_chance(25, 20)), (40, 50, 95));
        assert!((0..200).all(|_| !roll_spot(&rules, &keen, true, None, &mut rng)), "magical: not below Expert");
        let expert = sheet(5, EXPERT, SKILL_SPOT_TRAP);
        assert!((0..200).any(|_| roll_spot(&rules, &expert, true, None, &mut rng)));
    }

    #[test]
    fn a_master_disarms_more_often() {
        let mut rng = crate::HostRng::default();
        let won = |training, rng: &mut crate::HostRng| (0..2000).filter(|_| roll_disarm(50, training, None, rng) == crate::PickOutcome::Success).count();
        let (plain, master) = (won(0, &mut rng), won(MASTER, &mut rng));
        assert!((900..1100).contains(&plain), "{plain}");
        assert!(master > plain + 300, "{master} vs {plain}");
    }
}
