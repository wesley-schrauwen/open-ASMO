//! Items' hit points and the Repair skill (Arcanum.exe: item hit points 0x43d5a0 / 0x43d600,
//! combat wear 0x4b4390, Repair 0x4c7160 case 0xc at 0x4c7bce, tech chance 0x4c69f0).
//!
//! - **Hit points:** an item's most is `hp_pts + hp_adj` (fields 0x1b, 0x1c), less what repairs
//!   took ([`ItemWear::lost`]); what it lost is `hp_damage` (0x1d, [`ItemWear::damage`]).
//!   Carried and dropped items keep their wear.
//! - **Wear in a fight** ([`wear_item`], 0x4b4390): a critical hit's armour or weapon damage takes
//!   [`COMBAT_WEAR`] hit points; at 0 left the item is destroyed. Items with no hit points are
//!   left alone (builder's call). Nothing else is known to weaken a worn item.
//! - **Repair** ([`repair`]): on a carried item that lost hit points (else `skill_ui.mes` 505
//!   "not in need of repair"). No roll: with a chance ([`crate::tech_chance`]) of 1 or more it
//!   restores `max × chance / 100`, at least 1, and the most falls by 10 / 5 / 1 / 0 % by training
//!   (none, Apprentice, Expert, Master); 500 then; with no chance it fails, 501.
//!
//! Not built: a Master mending broken items (art bit 0x400, 5 %), the critical failure at Master
//! (1 %), effect 9 (everything restored), repairing items on the ground or other critters',
//! critical misses wearing the attacker's weapon (0x4c82e0), items immune to wear (flag
//! 0x400000).

use arcanum_formats::object::Prototypes;

use crate::{CarriedItem, Inventory, ItemWear};

/// The Repair skill.
pub const SKILL_REPAIR: usize = 12;
/// Hit points a critical hit's wear takes from an item (0x4b4390).
pub const COMBAT_WEAR: i32 = 10;
/// `mes/skill_ui.mes`: repaired, failed, not in need of repair.
pub const REPAIRED: u32 = 500;
pub const REPAIR_FAILED: u32 = 501;
pub const NOT_DAMAGED: u32 = 505;
/// The most hit points a repair takes, in percent, by training (0x4c7bce's table).
pub const REPAIR_LOSS: [i32; 4] = [10, 5, 1, 0];

/// An item's most hit points now: `hp_pts + hp_adj`, less what repairs took.
pub fn item_max_hp(item: &CarriedItem, prototypes: &Prototypes) -> i32 {
    let Some(o) = prototypes.get(item.prototype) else { return 0 };
    prototypes.int(o, "hp_pts").unwrap_or(0) + prototypes.int(o, "hp_adj").unwrap_or(0) - item.wear.lost
}

/// Wears the item at `location` by [`COMBAT_WEAR`]: `None` if there is none or it has no hit
/// points, else whether it broke (and was taken out).
pub fn wear_item(inventory: &mut Inventory, location: i32, prototypes: &Prototypes) -> Option<bool> {
    let i = inventory.items.iter().position(|i| i.location == location)?;
    let max = item_max_hp(&inventory.items[i], prototypes);
    if max <= 0 {
        return None;
    }
    let wear = &mut inventory.items[i].wear;
    wear.damage += COMBAT_WEAR;
    let broken = wear.damage >= max;
    if broken {
        inventory.items.remove(i);
    }
    Some(broken)
}

/// Repairs `wear` of an item of `max` hit points with `chance` at `training`: the new wear and
/// the `skill_ui.mes` line, or the line why not.
pub fn repair(wear: ItemWear, max: i32, chance: i32, training: u8) -> Result<(ItemWear, u32), u32> {
    if wear.damage < 1 {
        return Err(NOT_DAMAGED);
    }
    if chance < 1 {
        return Err(REPAIR_FAILED);
    }
    let amount = (max * chance / 100).max(1);
    let lost = max * REPAIR_LOSS[usize::from(training.min(3))] / 100;
    Ok((ItemWear { damage: (wear.damage - amount).max(0), lost: wear.lost + lost }, REPAIRED))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_restore_by_the_chance_and_cost_by_training() {
        let worn = ItemWear { damage: 30, lost: 0 };
        assert_eq!(repair(ItemWear::default(), 100, 50, 0), Err(NOT_DAMAGED));
        assert_eq!(repair(worn, 100, 0, 3), Err(REPAIR_FAILED));
        assert_eq!(repair(worn, 100, 20, 0), Ok((ItemWear { damage: 10, lost: 10 }, REPAIRED)));
        assert_eq!(repair(worn, 100, 50, 1), Ok((ItemWear { damage: 0, lost: 5 }, REPAIRED)));
        assert_eq!(repair(worn, 100, 50, 3), Ok((ItemWear { damage: 0, lost: 0 }, REPAIRED)));
        assert_eq!(repair(ItemWear { damage: 5, lost: 0 }, 10, 4, 2), Ok((ItemWear { damage: 4, lost: 0 }, REPAIRED)), "at least 1");
    }
}
