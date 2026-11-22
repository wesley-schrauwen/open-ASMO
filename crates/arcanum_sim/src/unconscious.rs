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
//! Not built: the spells being cast and maintained ending (0x457450, 0x4cbbf0; none are
//! maintained yet); the turn ending in turn-based combat (0x4b7c90).

use bevy_ecs::prelude::*;

use crate::{PlayerControlled, Sheet, SimRules, pc_status};

/// A PC lying unconscious.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unconscious;

/// Whether a PC with fatigue `current` is unconscious (0x45d800).
pub fn is_unconscious(current: i32) -> bool {
    current < 1
}

type ConsciousQuery<'a> = (Entity, &'a PlayerControlled, &'a Sheet, &'a crate::Position, Has<Unconscious>, Has<crate::Walking>);

/// Knocks down the PCs whose fatigue fell below 1 and gets up those above 0 again.
pub fn update_consciousness(mut commands: Commands, rules: Res<SimRules>, mut world: ResMut<crate::WorldState>, pcs: Query<ConsciousQuery>) {
    for (entity, player, sheet, position, down, walking) in &pcs {
        let now = is_unconscious(pc_status(&rules.0, &sheet.0).fatigue.current);
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

/// Drops what unconscious PCs' players ask them to do (0x4348e0).
pub fn gate_unconscious(
    pcs: Query<&PlayerControlled, With<Unconscious>>,
    mut moves: ResMut<crate::MoveRequests>,
    mut uses: ResMut<crate::UseRequests>,
    mut pickups: ResMut<crate::PickUpRequests>,
    mut picks: ResMut<crate::PickLockRequests>,
    mut casts: ResMut<crate::CastRequests>,
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
