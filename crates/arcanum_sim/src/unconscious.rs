//! Falling unconscious from fatigue (Arcanum.exe 0x45d5a0 / 0x45d800 / 0x435a90 / 0x45e610 /
//! 0x434de0).
//!
//! - **Falling** (0x45d5a0): when fatigue drops below 1 (0x45d800) the critter is knocked down
//!   (animation goal 62, art animation 7) and lies there; whatever it was doing stops (0x424070
//!   interrupts its goals). Here the PC stops walking and its waiting door use, pick-up and lock
//!   pick are dropped.
//! - **Lying** (0x4348e0 gates every animation goal on 0x45d790): it walks, uses, picks up,
//!   picks and casts nothing; those requests are dropped as they arrive ([`gate_unconscious`]).
//! - **Waking** (0x45e610 → 0x434de0): when the fatigue recovery takes fatigue above 0 the
//!   critter gets up (goal 63, art animation 8). No message and no sound either way.
//! - Clients see it in the PC's fatigue ([`crate::PcStatus`]): below 1 is down.
//!
//! - **Death** (owner's call for co-op; the original's PC dies at 0 hit points, 0x45d8d0 →
//!   0x460780, and the game ends): a PC whose hit points run out is dead
//!   (`CharacterSheet::dead`). It lies where it fell and takes no requests; healing, rest and
//!   scripts' healing do not raise it. Only resurrection does ([`Resurrections`]: a healer's
//!   Resurrect bought by another PC): it gets up with 15 % of its hit points (owner's call).
//!
//! Not built: the spells being cast and maintained ending (0x457450, 0x4cbbf0; none are
//! maintained yet); the turn ending in turn-based combat (0x4b7c90); moving a dead PC to the
//! nearest inn (owner's note: to be detailed); the Resurrect spell cast by a PC; a dead PC's
//! inventory and followers.

use bevy_ecs::prelude::*;

use crate::{PlayerControlled, Sheet, SimRules, pc_status};

/// A PC lying unconscious.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unconscious;

/// Whether a PC with fatigue `current` is unconscious (0x45d800).
pub fn is_unconscious(current: i32) -> bool {
    current < 1
}

/// Whether a PC lies down: unconscious, out of hit points, or dead.
pub fn is_down(status: &crate::PcStatus) -> bool {
    status.dead || is_unconscious(status.fatigue.current) || status.hit_points.current < 1
}

/// Resurrections bought this tick: every dead PC gets up.
#[derive(Resource, Debug, Default)]
pub struct Resurrections(pub u32);

/// The share of its most hit points a resurrected PC gets up with, in percent (owner's call: the
/// original never resurrects a PC, its death ends the game).
pub const RESURRECTED_PERCENT: i32 = 15;

/// Hit points a PC of `max` gets up with when resurrected: [`RESURRECTED_PERCENT`], at least 1.
pub fn resurrected_hp(max: i32) -> i32 {
    (max * RESURRECTED_PERCENT / 100).max(1)
}

/// Raises the dead PCs when a resurrection was bought ([`Resurrections`]).
pub fn resurrect_pcs(mut asked: ResMut<Resurrections>, rules: Res<SimRules>, mut world: ResMut<crate::WorldState>, mut sheets: ResMut<crate::ScriptSheets>) {
    if std::mem::take(&mut asked.0) == 0 {
        return;
    }
    let Some(save) = world.0.as_mut() else { return };
    for (&player, saved) in save.players.iter_mut().filter(|(_, s)| s.blueprint.sheet.dead) {
        let sheet = &mut saved.blueprint.sheet;
        sheet.dead = false;
        let max = rules.0.derived(sheet).hit_points;
        sheet.hp_damage = (max - resurrected_hp(max)).max(0);
        sheets.0.insert(player);
    }
}

type ConsciousQuery<'a> = (Entity, &'a PlayerControlled, &'a Sheet, &'a crate::Position, Has<Unconscious>, Has<crate::Walking>);

/// Knocks down the PCs whose fatigue or hit points fell below 1 and gets up those above 0
/// again.
pub fn update_consciousness(mut commands: Commands, rules: Res<SimRules>, mut world: ResMut<crate::WorldState>, mut sheets: ResMut<crate::ScriptSheets>, pcs: Query<ConsciousQuery>) {
    for (entity, player, sheet, position, down, walking) in &pcs {
        let status = pc_status(&rules.0, &sheet.0);
        // Out of hit points: dead (owner's call), kept on the sheet.
        if status.hit_points.current < 1
            && !sheet.0.dead
            && let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player.0))
        {
            saved.blueprint.sheet.dead = true;
            sheets.0.insert(player.0);
        }
        let now = is_down(&status);
        if now == down {
            continue;
        }
        let mut pc = commands.entity(entity);
        if now {
            pc.insert(Unconscious).remove::<(crate::Walking, crate::PendingUse, crate::PendingPick, crate::PendingPickUp)>();
            if walking {
                crate::record_position(&mut world, player.0, position.0);
            }
        } else {
            pc.remove::<Unconscious>();
        }
    }
}

/// PCs that take no requests: unconscious or resting.
type Idle = Or<(With<Unconscious>, With<crate::Resting>)>;

/// Drops what unconscious (0x4348e0) and resting ([`crate::rest`]) PCs' players ask them to do.
pub fn gate_unconscious(
    pcs: Query<&PlayerControlled, Idle>,
    mut moves: ResMut<crate::MoveRequests>,
    mut uses: ResMut<crate::UseRequests>,
    mut pickups: ResMut<crate::PickUpRequests>,
    (mut picks, mut casts): (ResMut<crate::PickLockRequests>, ResMut<crate::CastRequests>),
    mut disarms: ResMut<crate::DisarmRequests>,
    (mut rests, mut talks): (ResMut<crate::RestRequests>, ResMut<crate::TalkRequests>),
) {
    let down: Vec<crate::PlayerId> = pcs.iter().map(|p| p.0).collect();
    if down.is_empty() {
        return;
    }
    moves.0.retain(|r| !down.contains(&r.0));
    uses.0.retain(|r| !down.contains(&r.0));
    pickups.0.retain(|r| !down.contains(&r.0));
    picks.0.retain(|r| !down.contains(&r.0));
    casts.0.retain(|r| !down.contains(&r.0));
    disarms.0.retain(|r| !down.contains(&r.0));
    rests.0.retain(|r| !down.contains(&r.0));
    talks.talk.retain(|r| !down.contains(&r.0));
    talks.pockets.retain(|r| !down.contains(&r.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_one_fatigue_is_unconscious() {
        assert!(is_unconscious(0) && is_unconscious(-14));
        assert!(!is_unconscious(1));
    }
}
