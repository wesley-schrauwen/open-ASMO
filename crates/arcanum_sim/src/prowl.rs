//! Prowling: sneaking with the Prowling skill (Arcanum.exe toggle 0x57a0a0 case 0 → 0x45ee30 →
//! 0x45ee90; the skill's value 0x4c62e0; hearing 0x4af470).
//!
//! - **On and off:** the skills view's Prowling button toggles it (`PlayerIntent::Prowl`); a
//!   right click in the world turns it off (0x54de50). The original keeps it in `critter_flags`
//!   (0x1 | 0x2) and switches the critter's art to animation 5 ([`ANIM_PROWL`], 0x45efa0;
//!   off: animation 0): here the PC carries [`Prowling`], its [`crate::Appearance`] takes
//!   animation 5 (later players see it in the `Spawn`), and every player gets
//!   [`LedgerDelta::Prowling`]. It is not saved (builder's call).
//! - **Its value** ([`prowl_value`], 0x4c62e0 skill 6): 0 untrained by level, else level × 5 +
//!   25.
//! - **Hearing** ([`hearing_reach`], 0x4af470): a listener hears within `(clamp(PE - 4, 1, 10) +
//!   6) / 2` tiles; against a prowler its Perception first loses the prowler's value in percent
//!   (at most 100). The difficulty 0x4c8430 takes off (flag 0x2000) is not counted.
//!
//! - **Sight** ([`crate::combat::sees`], 0x4af260): within `clamp(PE, 1, 10)` tiles, Perception
//!   cut by a prowler's value in percent; in the watcher's facing cone; nothing between.
//!
//! Not built: invisibility, the light and the difficulty terms of a prowler's value (0x4c8430),
//! dropping prowling when alerted, followers copying their leader, untrained
//! prowlers' path flag 0x200, the hover text's prowling state.

use bevy_ecs::prelude::*;

use crate::{Appearance, LedgerDelta, NetId, Outbox, PlayerControlled, PlayerId, Session};

/// The Prowling skill.
pub const SKILL_PROWLING: usize = 6;
/// The critter animation of a prowler (0x45efa0).
pub const ANIM_PROWL: u32 = 5;

/// The PC is prowling.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prowling;

/// Prowl requests taken from the inbox: the player and on or off.
#[derive(Resource, Debug, Default)]
pub struct ProwlRequests(pub Vec<(PlayerId, bool)>);

/// The Prowling value of a prowler of skill `level` and `intelligence` (0x4c62e0: + 10 at
/// Intelligence 20, 0x4c6369).
pub fn prowl_value(level: i32, intelligence: i32) -> i32 {
    (if level <= 0 { 0 } else { level * 5 + 25 }) + crate::bright_bonus(intelligence)
}

/// How far a listener of `perception` hears, in tiles; `prowler` is the target's prowl value
/// when it prowls (0x4af470 with noise 1).
pub fn hearing_reach(perception: i32, prowler: Option<i32>) -> i64 {
    let mut perception = perception;
    if let Some(p) = prowler {
        perception -= perception * p.clamp(0, 100) / 100;
    }
    i64::from(((perception - 4).clamp(1, 10) + 6) / 2)
}

/// Turns prowling on or off as asked; every player in the world is told.
pub fn apply_prowling(mut commands: Commands, mut requests: ResMut<ProwlRequests>, session: Res<Session>, mut outbox: ResMut<Outbox>, mut pcs: Query<(Entity, &NetId, &PlayerControlled, &mut Appearance, Has<Prowling>)>) {
    for (player, on) in std::mem::take(&mut requests.0) {
        let Some((entity, &net_id, _, mut look, prowling)) = pcs.iter_mut().find(|pc| pc.2.0 == player) else { continue };
        if prowling == on {
            continue;
        }
        look.art = crate::gait::with_anim(look.art, if on { ANIM_PROWL } else { arcanum_formats::art_id::ANIM_STAND });
        if on {
            commands.entity(entity).insert(Prowling);
        } else {
            commands.entity(entity).remove::<Prowling>();
        }
        crate::ground::broadcast(&session, &mut outbox, LedgerDelta::Prowling { net_id, on });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prowlers_cut_the_listeners_perception() {
        assert_eq!((prowl_value(0, 8), prowl_value(1, 8), prowl_value(10, 8)), (0, 30, 75));
        assert_eq!(prowl_value(1, 20), 40, "Intelligence 20: + 10");
        assert_eq!(hearing_reach(10, None), 6, "(6 + 6) / 2");
        assert_eq!(hearing_reach(10, Some(50)), 3, "PE 5: (1 + 6) / 2");
        assert_eq!(hearing_reach(20, Some(200)), 3, "at most all of it");
    }
}
