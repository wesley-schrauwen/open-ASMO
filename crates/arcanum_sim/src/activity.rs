//! The distances that decide what the host simulates, copied from the original (owner's rule:
//! the host decides game state from positions and the game's rules, with the original's
//! distances).
//!
//! All distances are in tiles, measured as the larger of the two axis differences (0x4b96f0),
//! so "within N" is a square.
//!
//! - **NPC activity** ([`AI_RANGE`]): an NPC thinks (targets, wanders, flees) only while within
//!   30 tiles of a PC and no turn-based combat runs (AI heartbeat 0x4ad200, test 0x4ad420).
//!   Further away its goals are interrupted. Its next heartbeat comes sooner the nearer the PC
//!   is ([`ai_delay_ms`], 0x4ad5d0).
//! - **Combat** ([`combat_radius`], 0x4b7300): turn-based combat takes in every critter within
//!   `max(10, perception / 2 + 5)` of the PC who starts it.
//! - **Noticing** ([`notice_range`], 0x4ab460): an NPC looks for enemies within its
//!   perception, clamped to 1-10, and then needs to see them (0x4af470, not built).
//! - **Fidgets** (0x4300d0) run only for critters on screen: cosmetic, for the client.
//!
//! The original keeps no fixed block of sectors loaded: its 16-sector cache (0x4cf810, 0x4d0540)
//! loads what is touched. The host's block round each PC ([`crate::SECTOR_RADIUS`]) only has to
//! hold everything these rules look at, which [`AI_RANGE`] bounds.

use arcanum_formats::map::TileLoc;
use bevy_ecs::prelude::*;

use crate::{Npc, PlayerControlled, Position};

/// How near a PC an NPC must be to think (0x4ad420).
pub const AI_RANGE: u32 = 30;

/// Tile distance: the larger of the axis differences (0x4b96f0).
pub fn tile_distance(a: TileLoc, b: TileLoc) -> u32 {
    a.x.abs_diff(b.x).max(a.y.abs_diff(b.y))
}

/// Milliseconds to an NPC's next AI heartbeat when the nearest PC is `distance` tiles away:
/// 250 next to it, rising to 5000 at [`AI_RANGE`] or more (0x4ad5d0, 0x4ad610).
pub fn ai_delay_ms(distance: u32) -> u32 {
    250 + 4750 * distance.min(AI_RANGE) / AI_RANGE
}

/// The radius turn-based combat takes critters in from, for a PC with `perception` (0x4b7300).
pub fn combat_radius(perception: i32) -> u32 {
    (perception / 2 + 5).max(10) as u32
}

/// How far an NPC with `perception` looks for enemies (0x4ab460, 0x4af240).
pub fn notice_range(perception: i32) -> u32 {
    perception.clamp(1, 10) as u32
}

/// An NPC within [`AI_RANGE`] of a PC: the original runs its AI.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Active;

/// Marks the NPCs within [`AI_RANGE`] of any PC [`Active`] and unmarks the rest.
pub fn update_activity(
    mut commands: Commands,
    pcs: Query<&Position, With<PlayerControlled>>,
    npcs: Query<(Entity, &Position, Has<Active>), With<Npc>>,
) {
    let pcs: Vec<TileLoc> = pcs.iter().map(|p| p.0).collect();
    for (entity, &Position(at), active) in &npcs {
        let near = pcs.iter().any(|&pc| tile_distance(pc, at) <= AI_RANGE);
        if near && !active {
            commands.entity(entity).insert(Active);
        } else if !near && active {
            commands.entity(entity).remove::<Active>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(x: u32, y: u32) -> TileLoc {
        TileLoc { x, y }
    }

    #[test]
    fn distances_are_squares() {
        assert_eq!(tile_distance(t(10, 10), t(13, 40)), 30);
        assert_eq!(tile_distance(t(10, 10), t(40, 40)), 30, "a diagonal counts as its longer side");
    }

    #[test]
    fn the_originals_formulas() {
        assert_eq!(ai_delay_ms(0), 250);
        assert_eq!(ai_delay_ms(15), 2625);
        assert_eq!(ai_delay_ms(30), 5000);
        assert_eq!(ai_delay_ms(99), 5000);
        assert_eq!(combat_radius(8), 10, "never under 10");
        assert_eq!(combat_radius(20), 15);
        assert_eq!(notice_range(0), 1);
        assert_eq!(notice_range(8), 8);
        assert_eq!(notice_range(14), 10);
    }

    #[test]
    fn the_loaded_block_holds_everything_the_rules_look_at() {
        // A PC anywhere in its sector has at least SECTOR_RADIUS whole sectors loaded on each
        // side: every tile an NPC could think within is loaded.
        assert!(crate::SECTOR_RADIUS * arcanum_formats::map::SECTOR_TILES >= AI_RANGE);
        // Perception tops out at 20.
        assert!(crate::SECTOR_RADIUS * arcanum_formats::map::SECTOR_TILES >= combat_radius(20));
    }
}
