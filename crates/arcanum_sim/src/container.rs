//! The map's containers: chests, barrels, crates and the like (object type 2), opened into the
//! loot window (Arcanum.exe 0x572240 mode 2).
//!
//! - **Opening** (0x4aed80, through 0x572370): jammed refuses ("You cannot open that.",
//!   `mes/inven_ui.mes` 2), magically held refuses ("This seems to be stuck.", 3), locked
//!   ([`crate::locked`], the doors' test 0x441d40 on `container_flags`) refuses ("Locked.", 1)
//!   unless the PC carries its key (`container_key_id`, [`crate::keys`]); opening with the key
//!   leaves it locked.
//! - **Contents:** the map's stored list (`container_inventory_list_idx`), each item where it
//!   lies in the container's 10 x 96 grid (`item_inv_location`). A container with
//!   [`OCOF_INVEN_SPAWN_PENDING`] rolls its inventory source (`Rules/InvenSource.mes`, 0x463e20)
//!   on its first opening, added to what it holds: gold in the source's range, then each item by
//!   its chance (a roll of 1-100 at or under it), each placed first-fit; items that do not fit
//!   are dropped, as the original destroys them. The flag is then spent, so it happens once.
//! - **The open frame** (0x43f7f0 / 0x43f8f0): a container shown at frame 0 shows frame 1
//!   while a loot window is on it, and frame 0 again when the last one closes, unless it is
//!   `STICKY` (0x100): then it stays open.
//! - An empty container stays (the pile collapse, 0x463540, is for junk piles only).
//!
//! Not built: lockpicking, the open, close and locked sounds (`container_sound`
//! plus 0, 1, 2), traps, scripts, stealing (mode 4), and the original's seed for the roll: it mixes
//! the container's x with the opening PC's name and two stats (0x463e20, decoded with medium
//! confidence); here the seed is the container's tile, so each container rolls the same for
//! everyone.

use arcanum_formats::map::TileLoc;
use arcanum_rules::InventorySource;

use crate::{Inventory, ItemInfo};

/// `container_flags` (Arcanum.exe's `OCOF_` names at 0x5ba23c, in bit order).
pub const OCOF_LOCKED: i32 = 0x1;
pub const OCOF_JAMMED: i32 = 0x2;
pub const OCOF_MAGICALLY_HELD: i32 = 0x4;
pub const OCOF_NEVER_LOCKED: i32 = 0x8;
pub const OCOF_BUSTED: i32 = 0x80;
/// Stays open when the loot window closes.
pub const OCOF_STICKY: i32 = 0x100;
/// The inventory source has not been rolled yet (0x463e20 tests and clears it).
pub const OCOF_INVEN_SPAWN_PENDING: i32 = 0x200;

/// Why a container would not open: its `mes/inven_ui.mes` line (0x572370 shows line = status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRefusal {
    Locked = 1,
    Jammed = 2,
    MagicallyHeld = 3,
}

impl OpenRefusal {
    pub fn message(self) -> u32 {
        self as u32
    }
}

/// Whether a container with `flags` opens (0x4aed80), with day locks holding or not (`day`),
/// for a PC with its key or not (`key`).
pub fn open_check(flags: i32, day: bool, key: bool) -> Result<(), OpenRefusal> {
    if flags & OCOF_JAMMED != 0 {
        return Err(OpenRefusal::Jammed);
    }
    if flags & OCOF_MAGICALLY_HELD != 0 {
        return Err(OpenRefusal::MagicallyHeld);
    }
    if crate::locked(flags, day) && !key {
        return Err(OpenRefusal::Locked);
    }
    Ok(())
}

/// The original's random numbers (0x4cef50): `seed * 0x19660D + 0x3C6EF35F`, bits 8-22.
#[derive(Debug, Clone, Copy)]
pub struct Rng(pub u32);

impl Rng {
    pub fn roll(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x0019_660D).wrapping_add(0x3C6E_F35F);
        (self.0 >> 8) & 0x7FFF
    }

    /// A number from `a` to `b` inclusive, either way round (0x4cef00).
    pub fn range(&mut self, a: i32, b: i32) -> i32 {
        let (lo, hi) = if a > b { (b, a) } else { (a, b) };
        let span = (hi - lo + 1).max(1) as u32;
        (self.roll() % span) as i32 + lo
    }
}

/// The seed a container's roll starts from: its tile (builder's call, see the module notes).
pub fn roll_seed(at: TileLoc) -> u32 {
    at.x.wrapping_add(at.y.wrapping_mul(31))
}

/// Adds what `source` rolls to `contents` (0x463e20): gold, then each item by its chance, as
/// object numbers ([`crate::BASIC_TO_PROTOTYPE`] added to the file's basic numbers).
pub fn roll_source(contents: &mut Inventory, source: &InventorySource, rng: &mut Rng, info: impl Fn(u32) -> Option<ItemInfo>) {
    let gold = rng.range(source.gold.0, source.gold.1);
    if gold > 0 {
        let _ = contents.insert(crate::GOLD, gold, &info, i32::MAX, 0);
    }
    for &(chance, basic) in &source.items {
        if rng.range(1, 100) <= chance as i32 {
            let _ = contents.insert(basic + crate::BASIC_TO_PROTOTYPE, 1, &info, i32::MAX, 0);
        }
    }
}

/// The loot window's portrait (`art/interface/`, drawn at (183, 33)) for a container of
/// `prototype` (0x553d10, by basic number): junk pile 832, body 833, safes 834, trash 835, plant
/// 836, the good, neutral and evil altars 837-839, anything else `ContainerIcon` (435).
pub fn portrait(prototype: u32) -> &'static str {
    match prototype.wrapping_sub(crate::BASIC_TO_PROTOTYPE) {
        3023 => "Cont_Junk.ART",
        3056 => "Cont_Body.ART",
        3033 | 3034 => "Cont_Safe.ART",
        3055 => "Cont_Trash.ART",
        3060 => "Cont_Plant.ART",
        3057 => "Cont_Altar_Good.ART",
        3058 => "Cont_Altar_Neutral.ART",
        3059 => "Cont_Altar_Evil.ART",
        _ => "ContainerIcon.ART",
    }
}

/// Inventory source `id` of `Rules/InvenSource.mes`, if it can be read.
pub fn inventory_source(vfs: &arcanum_vfs::Vfs, id: u32) -> Option<InventorySource> {
    let data = vfs.read("Rules/InvenSource.mes").ok()?;
    let file = arcanum_formats::mes::MesFile::parse(&data).ok()?;
    arcanum_rules::parse_inventory_source(file.get(id)?)
}

/// Whether a container with art `art` shows open as frame 1: only one shown at frame 0 turns
/// (0x572640).
pub fn opens_to_frame(art: arcanum_formats::art_id::ArtId) -> bool {
    crate::portal_frame(art) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jammed_and_held_come_before_the_lock() {
        assert_eq!(open_check(0, true, false), Ok(()));
        assert_eq!(open_check(OCOF_LOCKED, true, false), Err(OpenRefusal::Locked));
        assert_eq!(open_check(OCOF_LOCKED, true, true), Ok(()), "the key opens it");
        assert_eq!(open_check(OCOF_LOCKED | OCOF_JAMMED, true, true), Err(OpenRefusal::Jammed));
        assert_eq!(open_check(OCOF_LOCKED | OCOF_MAGICALLY_HELD, true, true), Err(OpenRefusal::MagicallyHeld));
        // Never-locked and busted containers open; the real data's 0x9 is never locked.
        assert_eq!(open_check(OCOF_LOCKED | OCOF_NEVER_LOCKED, true, false), Ok(()));
        assert_eq!(open_check(OCOF_LOCKED | OCOF_BUSTED, true, false), Ok(()));
        assert_eq!(OpenRefusal::Locked.message(), 1);
        assert_eq!(OpenRefusal::MagicallyHeld.message(), 3);
    }

    #[test]
    fn portraits_go_by_prototype() {
        assert_eq!(portrait(crate::PILE), "Cont_Junk.ART");
        assert_eq!(portrait(3054), "Cont_Safe.ART");
        assert_eq!(portrait(3079), "Cont_Altar_Evil.ART");
        assert_eq!(portrait(3065), "ContainerIcon.ART");
    }

    #[test]
    fn the_generator_is_the_originals() {
        let mut rng = Rng(0);
        // seed = 0x3C6EF35F after one step; (seed >> 8) & 0x7FFF.
        assert_eq!(rng.roll(), (0x3C6E_F35Fu32 >> 8) & 0x7FFF);
        let mut rng = Rng(12345);
        for _ in 0..1000 {
            let n = rng.range(5, 2);
            assert!((2..=5).contains(&n));
        }
        assert_eq!(Rng(7).range(3, 3), 3);
    }

    #[test]
    fn a_source_gives_gold_and_items_by_chance() {
        use arcanum_formats::object::{TYPE_GENERIC, TYPE_GOLD};
        let info = |n: u32| {
            let kind = if n == crate::GOLD { TYPE_GOLD } else { TYPE_GENERIC };
            Some(ItemInfo { kind, inventory_art: 0, cells: (1, 1), weight: 1, magic_weight_adj: 0, complexity: 0, generic_flags: 0, weapon_flags: 0, armor_flags: 0, item_flags: 0 })
        };
        let source = InventorySource { gold: (10, 10), items: vec![(100, 1), (0, 2), (100, 3)] };
        let mut contents = Inventory::container();
        roll_source(&mut contents, &source, &mut Rng(1), info);
        let held: Vec<(u32, i32)> = contents.items.iter().map(|i| (i.prototype, i.quantity)).collect();
        assert_eq!(held, [(crate::GOLD, 10), (21, 0), (23, 0)]);
    }
}
