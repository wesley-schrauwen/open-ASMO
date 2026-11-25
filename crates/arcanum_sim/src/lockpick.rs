//! Picking locks: the Pick Locks skill used on a door or container (Arcanum.exe 0x4c7160 action
//! 0xe, chance 0x4c69f0, difficulty 0x4c8430, critical failure 0x4c82e0).
//!
//! - **Chance** (0x4c69f0): the skill's level (0-20) x 6, plus 10 with a base Intelligence (the
//!   engine's stat 4) of 20 or more, capped at 95. The original scales the PC's chance by the
//!   game difficulty (easy +50 %, hard -25 %); there is no difficulty setting yet, so it is the
//!   normal game's, unscaled.
//! - **Difficulty** (0x4c8430): the lock's (`portal_lock_difficulty`, `container_lock_difficulty`)
//!   when above 0, halved for a Master; less the tool's `generic_usage_bonus` (a lockpick:
//!   generic item with `generic_flags` 2).
//! - **Roll:** 1-100 plus the difficulty at or under the chance succeeds. A failure is critical
//!   when a second roll is at or under `max((100 - chance) / 2, 2)`, and jams the lock.
//! - **Success** on a locked target ([`crate::locked`]) clears the locked bit; on one not locked
//!   it sets it: picking an unlocked door or chest locks it (copied). The bit alone counts: a day
//!   or night lock holds by the hour whatever it is.
//! - **Results:** `mes/skill_ui.mes` 550 on success, 551 on failure (the original speaks them
//!   over the PC; here they go to the message box).
//! - The key number plays no part: locks no key opens can be picked. Jammed and magically held
//!   targets still refuse to open.
//! - **Timers left out:** the original queues an event an hour after a pick and a day after a
//!   jam (0x441dd0 / 0x441f10), but never writes the mode the event applies (0x4eec80 reads
//!   stale stack data), so what it does is undefined; here a picked lock stays picked and a jam
//!   stays.
//!
//! - **The tool** is the lockpick the player armed the pick with (the inventory's use box), its
//!   bonus counted while the PC still carries it; none, no bonus.
//!
//! Not built: the pick animation and its time, the darkness and critter-flag modifiers, the
//! witnesses (0x4aee50: the owner's faction or guards within 10 tiles who see it turn hostile),
//! the difficulty setting, followers picking.

use arcanum_formats::object::{Object, Prototypes, TYPE_CONTAINER, TYPE_GENERIC, TYPE_PORTAL};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    GroundItemId, HostRng, Inventory, LedgerDelta, LoadedMap, MapObjectId, NetId, Outbox,
    PlayerControlled, PlayerId, Position, Session, Sheet, SimRules, WorldState,
};

/// The Pick Locks skill (`mes/skill.mes` order).
pub const SKILL_PICK_LOCKS: usize = 14;
/// `generic_flags`: a lockpick (`OGF_IS_LOCKPICK`).
pub const OGF_IS_LOCKPICK: i32 = 0x2;
/// The highest chance.
pub const MAX_PICK_CHANCE: i32 = 95;
/// `mes/skill_ui.mes`: the lock opened.
pub const PICK_SUCCEEDED: u32 = 550;
/// `mes/skill_ui.mes`: the pick failed.
pub const PICK_FAILED: u32 = 551;
/// Training rank of a Master.
const MASTER: u8 = 3;

/// A door or container with a lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LockTarget {
    Door(MapObjectId),
    /// [`GroundItemId::Container`].
    Container(GroundItemId),
}

/// The chance in percent for a picker of skill `level` and base Intelligence `intelligence`.
pub fn pick_chance(level: i32, intelligence: i32) -> i32 {
    let bonus = if intelligence >= 20 { 10 } else { 0 };
    (level * 6 + bonus).clamp(0, MAX_PICK_CHANCE)
}

/// The difficulty of a lock of `lock` difficulty, for a picker of `training` rank with a tool
/// of `tool` bonus.
pub fn pick_difficulty(lock: i32, training: u8, tool: i32) -> i32 {
    let lock = if lock > 0 && training >= MASTER {
        lock / 2
    } else {
        lock.max(0)
    };
    lock - tool
}

/// How a pick went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickOutcome {
    Success,
    Failure,
    /// Failed and jammed the lock.
    Jammed,
}

/// Rolls a pick at `chance` against `difficulty`.
pub fn roll_pick(chance: i32, difficulty: i32, rng: &mut HostRng) -> PickOutcome {
    if rng.range(1, 100) + difficulty <= chance {
        return PickOutcome::Success;
    }
    if rng.range(1, 100) <= ((100 - chance) / 2).max(2) {
        PickOutcome::Jammed
    } else {
        PickOutcome::Failure
    }
}

/// Whether items of `prototype` are lockpicks.
pub fn is_lockpick(prototype: u32, prototypes: &Prototypes) -> bool {
    prototypes.get(prototype).is_some_and(|p| {
        p.object_type == TYPE_GENERIC && p.int("generic_flags").unwrap_or(0) & OGF_IS_LOCKPICK != 0
    })
}

/// The bonus of the lockpick at location `tool` of `inventory`, 0 for none or not a lockpick.
pub fn lockpick_bonus(inventory: &Inventory, tool: Option<i32>, prototypes: &Prototypes) -> i32 {
    let Some(item) = tool.and_then(|t| inventory.items.iter().find(|i| i.location == t)) else {
        return 0;
    };
    if !is_lockpick(item.prototype, prototypes) {
        return 0;
    }
    prototypes
        .get(item.prototype)
        .and_then(|p| p.int("generic_usage_bonus"))
        .unwrap_or(0)
}

/// The flag field of a lock target's kind.
fn flags_field(kind: u32) -> &'static str {
    if kind == TYPE_PORTAL {
        "portal_flags"
    } else {
        "container_flags"
    }
}

impl LoadedMap {
    /// The object a lock target names.
    pub fn lock_object(&self, target: LockTarget) -> Option<&Object> {
        match target {
            LockTarget::Door(id) => self.map_object(id).filter(|o| o.object_type == TYPE_PORTAL),
            LockTarget::Container(id) => self
                .container_object(id)
                .filter(|o| o.object_type == TYPE_CONTAINER),
        }
    }

    /// A lock target's flags (`portal_flags` or `container_flags`).
    pub fn lock_flags(&self, target: LockTarget) -> Option<i32> {
        let object = self.lock_object(target)?;
        Some(
            self.prototypes
                .int(object, flags_field(object.object_type))
                .unwrap_or(0),
        )
    }

    /// Sets a lock target's flags, in [`LoadedMap::changes`] too.
    pub fn set_lock_flags(&mut self, target: LockTarget, flags: i32) {
        let (sector, index) = match target {
            LockTarget::Door(id) => (id.sector, id.index),
            LockTarget::Container(id) => (
                id.sector(),
                match id {
                    GroundItemId::Container { index, .. } => index,
                    _ => return,
                },
            ),
        };
        let changes = self.changes.entry(sector).or_default();
        match target {
            LockTarget::Door(_) => changes.portal_flags.insert(index, flags),
            LockTarget::Container(_) => changes.container_flags.insert(index, flags),
        };
        self.apply_lock_flags(target, flags);
    }

    /// Writes a lock target's flags onto its object and works out its sector's blocking again.
    pub(crate) fn apply_lock_flags(&mut self, target: LockTarget, flags: i32) {
        let (sector, object) = match target {
            LockTarget::Door(id) => (
                id.sector,
                self.sectors
                    .get_mut(&id.sector)
                    .and_then(|s| s.objects.get_mut(id.index as usize)),
            ),
            LockTarget::Container(GroundItemId::Container { sector, index }) => (
                sector,
                self.mobiles
                    .get_mut(&sector)
                    .and_then(|m| m.get_mut(index as usize)),
            ),
            LockTarget::Container(_) => return,
        };
        let Some(object) = object else { return };
        let field = flags_field(object.object_type);
        object.set_int(field, flags);
        self.rebuild_blocking(sector);
    }

    /// The lock difficulty of a target.
    pub fn lock_difficulty(&self, target: LockTarget) -> i32 {
        let Some(object) = self.lock_object(target) else {
            return 0;
        };
        let field = if object.object_type == TYPE_PORTAL {
            "portal_lock_difficulty"
        } else {
            "container_lock_difficulty"
        };
        self.prototypes.int(object, field).unwrap_or(0)
    }

    /// Where a lock target stands.
    pub fn lock_location(&self, target: LockTarget) -> Option<arcanum_formats::map::TileLoc> {
        crate::object_location(self.lock_object(target)?)
    }
}

/// The flags after a pick with `outcome` on a lock with `flags`, at `day` or not.
pub fn picked_flags(flags: i32, outcome: PickOutcome, day: bool) -> i32 {
    match outcome {
        PickOutcome::Success if crate::locked(flags, day) => flags & !crate::OPF_LOCKED,
        PickOutcome::Success => flags | crate::OPF_LOCKED,
        PickOutcome::Jammed => flags | crate::OPF_JAMMED,
        PickOutcome::Failure => flags,
    }
}

/// Pick requests taken from the inbox by `handle_intents`: the player, the lock and the move.
#[derive(Resource, Debug, Default)]
pub struct PickLockRequests(pub Vec<(PlayerId, LockTarget, Option<i32>, u32)>);

/// A PC's pick waiting for it to walk in range. `seq` is the move it came with.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingPick {
    pub target: LockTarget,
    pub tool: Option<i32>,
    pub seq: u32,
}

type PickerQuery<'a> = (
    Entity,
    &'a NetId,
    &'a PlayerControlled,
    &'a Position,
    Option<&'a crate::Walking>,
    Option<&'a PendingPick>,
    &'a Sheet,
    &'a Inventory,
);

/// Takes pick requests (each becomes the PC's [`PendingPick`]), then picks the locks whose PC
/// is within [`crate::USE_RANGE`]; a request whose PC stopped out of range is dropped.
#[allow(clippy::too_many_arguments)]
pub fn pick_locks(
    mut commands: Commands,
    mut requests: ResMut<PickLockRequests>,
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    mut rng: ResMut<HostRng>,
    session: Res<Session>,
    rules: Res<SimRules>,
    mut outbox: ResMut<Outbox>,
    pcs: Query<PickerQuery>,
) {
    let mut fresh = std::collections::HashMap::new();
    for (player, target, tool, seq) in requests.0.drain(..) {
        if let Some((entity, ..)) = pcs.iter().find(|pc| pc.2.0 == player) {
            fresh.insert(entity, PendingPick { target, tool, seq });
        }
    }
    for (entity, _, controlled, position, walking, pending, sheet, inventory) in &pcs {
        let Some(want) = fresh.get(&entity).copied().or(pending.copied()) else {
            continue;
        };
        let Some(location) = map.lock_location(want.target) else {
            commands.entity(entity).remove::<PendingPick>();
            continue;
        };
        if !crate::within_radius(position.0, location, crate::USE_RANGE) {
            if walking.is_some() {
                commands.entity(entity).insert(want);
            } else {
                commands.entity(entity).remove::<PendingPick>();
            }
            continue;
        }
        commands.entity(entity).remove::<PendingPick>();
        let Some(flags) = map.lock_flags(want.target) else {
            continue;
        };
        let sheet = &sheet.0;
        let chance = pick_chance(rules.0.skill_level(sheet, SKILL_PICK_LOCKS), sheet.base[4]);
        let difficulty = pick_difficulty(
            map.lock_difficulty(want.target),
            sheet.training[SKILL_PICK_LOCKS],
            lockpick_bonus(inventory, want.tool, &map.prototypes),
        );
        let outcome = roll_pick(chance, difficulty, &mut rng);
        let changed = picked_flags(flags, outcome, map.lock_day);
        if changed != flags {
            map.set_lock_flags(want.target, changed);
            let sector = match want.target {
                LockTarget::Door(id) => id.sector,
                LockTarget::Container(id) => id.sector(),
            };
            crate::ground::save_change(&mut world, &map, sector);
            crate::ground::broadcast(
                &session,
                &mut outbox,
                LedgerDelta::LockChanged {
                    target: want.target,
                    flags: changed,
                },
            );
        }
        let message = if outcome == PickOutcome::Success {
            PICK_SUCCEEDED
        } else {
            PICK_FAILED
        };
        outbox
            .0
            .push((controlled.0, LedgerDelta::SkillResult { message }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chance_is_six_a_level_capped_at_95() {
        assert_eq!(pick_chance(0, 8), 0);
        assert_eq!(pick_chance(4, 8), 24);
        assert_eq!(pick_chance(4, 20), 34, "an extraordinary Intelligence");
        assert_eq!(pick_chance(20, 20), 95);
    }

    #[test]
    fn a_master_halves_the_lock_and_a_tool_takes_off_its_bonus() {
        assert_eq!(pick_difficulty(40, 0, 0), 40);
        assert_eq!(pick_difficulty(40, 2, 0), 40, "an expert does not");
        assert_eq!(pick_difficulty(41, 3, 0), 20);
        assert_eq!(pick_difficulty(40, 0, 10), 30, "the skeleton key");
        assert_eq!(pick_difficulty(-5, 3, 5), -5, "a negative lock counts as 0");
    }

    #[test]
    fn a_pick_unlocks_a_locked_target_and_locks_an_unlocked_one() {
        use crate::{OPF_JAMMED, OPF_LOCKED, OPF_LOCKED_NIGHT};
        assert_eq!(picked_flags(OPF_LOCKED, PickOutcome::Success, true), 0);
        assert_eq!(
            picked_flags(0, PickOutcome::Success, true),
            OPF_LOCKED,
            "copied: it locks"
        );
        assert_eq!(
            picked_flags(OPF_LOCKED, PickOutcome::Jammed, true),
            OPF_LOCKED | OPF_JAMMED
        );
        assert_eq!(
            picked_flags(OPF_LOCKED, PickOutcome::Failure, true),
            OPF_LOCKED
        );
        // A night lock by night: the bit goes, the hour still locks it.
        let night = OPF_LOCKED | OPF_LOCKED_NIGHT;
        assert_eq!(
            picked_flags(night, PickOutcome::Success, false),
            OPF_LOCKED_NIGHT
        );
        assert!(crate::locked(OPF_LOCKED_NIGHT, false));
    }

    #[test]
    fn only_the_armed_lockpick_gives_its_bonus() {
        use arcanum_formats::object::{Object, TYPE_WEAPON, Value};
        use arcanum_formats::test_util::{ObjectBuilder, prototype_id};
        let proto = |number: u32, kind: u32, flags: i32, bonus: i32| {
            let b = ObjectBuilder::new(kind);
            let b = if kind == TYPE_GENERIC {
                b.field("generic_flags", Value::Int32(flags))
                    .field("generic_usage_bonus", Value::Int32(bonus))
            } else {
                b
            };
            let mut o = Object::parse_instance(&b.build()).unwrap();
            o.id = prototype_id(number);
            o
        };
        let mut prototypes = Prototypes::default();
        prototypes.insert(proto(15101, TYPE_GENERIC, OGF_IS_LOCKPICK, 10));
        prototypes.insert(proto(15200, TYPE_GENERIC, 8, 20));
        prototypes.insert(proto(6049, TYPE_WEAPON, 0, 0));
        let item = |prototype, location| crate::CarriedItem {
            prototype,
            location,
            quantity: 0,
            keys: vec![],
        };
        let inv = Inventory {
            items: vec![item(15101, 0), item(15200, 1), item(6049, 2)],
            ..Default::default()
        };
        assert_eq!(
            lockpick_bonus(&inv, Some(0), &prototypes),
            10,
            "the auto skeleton key"
        );
        assert_eq!(
            lockpick_bonus(&inv, Some(1), &prototypes),
            0,
            "a healing kit is no lockpick"
        );
        assert_eq!(lockpick_bonus(&inv, Some(2), &prototypes), 0);
        assert_eq!(
            lockpick_bonus(&inv, Some(5), &prototypes),
            0,
            "no longer carried"
        );
        assert_eq!(lockpick_bonus(&inv, None, &prototypes), 0);
        let barred = Inventory {
            items: vec![item(15101, 2004)],
            ..Default::default()
        };
        assert_eq!(
            lockpick_bonus(&barred, Some(2004), &prototypes),
            10,
            "armed from the hotkey bar"
        );
        assert!(is_lockpick(15101, &prototypes) && !is_lockpick(15200, &prototypes));
    }

    #[test]
    fn rolls_follow_the_chance() {
        let mut rng = HostRng::default();
        let picks: Vec<PickOutcome> = (0..2000).map(|_| roll_pick(50, 0, &mut rng)).collect();
        let won = picks.iter().filter(|&&p| p == PickOutcome::Success).count();
        assert!((900..1100).contains(&won), "{won}");
        assert!(picks.contains(&PickOutcome::Jammed));
        assert!(
            (0..200).all(|_| roll_pick(95, 100, &mut rng) != PickOutcome::Success),
            "too hard"
        );
        assert!(
            (0..200).all(|_| roll_pick(0, -100, &mut rng) == PickOutcome::Success),
            "a negative difficulty wins at 0 %"
        );
    }
}
