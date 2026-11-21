//! What a critter carries (Arcanum.exe's item module: insert 0x461810 / 0x466640, the free-spot
//! search 0x4675a0, weight 0x461370, encumbrance 0x45f790).
//!
//! - **Locations:** `row * 10 + column` in a grid of [`COLUMNS`] x [`PC_ROWS`] 32-pixel cells;
//!   1000-1008 are the paperdoll slots ([`Slot`]); 2000-2009 the hotkey bar. Only grid items
//!   take cells.
//! - **Footprint:** the item's inventory art (`item_inv_aid`), `ceil(pixels / 32)` cells each
//!   way; 1x1 when the art cannot be read (0x461780).
//! - **Placement:** the first free rectangle, rows top to bottom, columns left to right
//!   (0x4675a0). Gold and ammo first join a stack of the same prototype (0x462930). An item that
//!   does not fit, or that would take the total weight past the carry weight, is refused.
//! - **Weight** (0x461370): gold weighs nothing; `item_weight`, plus `item_magic_weight_adj`
//!   scaled by how the carrier's magick/tech aptitude meets the item's complexity (0x461590);
//!   ammo `weight * quantity / 4`.
//! - **Encumbrance** (0x45f790): the first level whose share of the carry weight the total is
//!   below, out of 15, 30, 45, 60, 75, 90 and 100 %; 6 above 90 %.
//!
//! Not built: keys going onto the key ring (0x466a00), item effects on the carrier, the
//! encumbrance penalty (effect 0x9e, 0x45f820), the weapon scoring auto-equip uses (0x465170).

use arcanum_formats::art_id::ArtId;
use arcanum_formats::object::{Object, Prototypes, TYPE_AMMO, TYPE_ARMOR, TYPE_GENERIC, TYPE_GOLD, TYPE_WEAPON};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

/// Grid columns.
pub const COLUMNS: i32 = 10;
/// Grid rows of a critter (containers have 96).
pub const PC_ROWS: i32 = 12;
/// Ammo kinds (arrows, bullets, charges, fuel: `ammo_type` 0-3).
pub const AMMO_TYPES: usize = 4;
/// Cell size in pixels.
pub const CELL: u32 = 32;
/// The first paperdoll slot.
pub const FIRST_SLOT: i32 = 1000;
/// The last paperdoll slot.
pub const LAST_SLOT: i32 = 1008;
/// The hotkey bar's locations (no cells).
pub const HOTKEYS: std::ops::RangeInclusive<i32> = 2000..=2009;
/// Encumbrance thresholds, in percent of the carry weight (0x5b306c).
pub const ENCUMBRANCE_PERCENT: [i32; 7] = [15, 30, 45, 60, 75, 90, 100];

/// The paperdoll slots, by location (table 0x5cac58; item to slot 0x4655d0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Helmet = 1000,
    Ring1 = 1001,
    Ring2 = 1002,
    Amulet = 1003,
    Weapon = 1004,
    Shield = 1005,
    Armor = 1006,
    Gauntlets = 1007,
    Boots = 1008,
}

/// Whether `location` is a grid cell (not a slot or the hotkey bar).
pub fn in_grid(location: i32) -> bool {
    (0..COLUMNS * PC_ROWS).contains(&location)
}

/// One carried item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarriedItem {
    pub prototype: u32,
    pub location: i32,
    /// Gold and ammo: how many; 0 for everything else.
    #[serde(default)]
    pub quantity: i32,
}

/// What a critter carries.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<CarriedItem>,
}

/// What placement and weight need of an item's prototype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemInfo {
    /// The object type (`TYPE_WEAPON`, ...).
    pub kind: u32,
    pub inventory_art: u32,
    /// Cells taken, across and down.
    pub cells: (i32, i32),
    pub weight: i32,
    pub magic_weight_adj: i32,
    pub complexity: i32,
    /// Generic items' flags (bit 0: held in the shield hand).
    pub generic_flags: i32,
}

/// Cells for an art of `width` x `height` pixels: `((n - 1) >> 5) + 1` each way.
pub fn cells_for(width: u32, height: u32) -> (i32, i32) {
    let c = |n: u32| (n.max(1) - 1) as i32 / CELL as i32 + 1;
    (c(width), c(height))
}

impl ItemInfo {
    /// Reads `prototype`; `art_size` gives an art's first frame in pixels.
    pub fn of(prototype: &Object, art_size: impl Fn(ArtId) -> Option<(u32, u32)>) -> Self {
        let int = |name: &str| prototype.int(name).unwrap_or(0);
        let inventory_art = int("item_inv_aid") as u32;
        let cells = art_size(ArtId(inventory_art)).map_or((1, 1), |(w, h)| cells_for(w, h));
        Self {
            kind: prototype.object_type,
            inventory_art,
            cells,
            weight: int("item_weight"),
            magic_weight_adj: int("item_magic_weight_adj"),
            complexity: int("item_magic_tech_complexity"),
            generic_flags: int("generic_flags"),
        }
    }

    /// Gold and ammo stack.
    pub fn stacks(&self) -> bool {
        self.kind == TYPE_GOLD || self.kind == TYPE_AMMO
    }

    /// The slot it is worn in (0x4655d0): weapons in the weapon hand, armour by the coverage in
    /// its inventory art (bits 14-16), shield-hand generics; anything else none.
    pub fn slot(&self) -> Option<Slot> {
        match self.kind {
            TYPE_WEAPON => Some(Slot::Weapon),
            TYPE_ARMOR => match (self.inventory_art >> 14) & 7 {
                2 => Some(Slot::Helmet),
                5 => Some(Slot::Ring1),
                6 => Some(Slot::Amulet),
                1 => Some(Slot::Shield),
                0 => Some(Slot::Armor),
                3 => Some(Slot::Gauntlets),
                4 => Some(Slot::Boots),
                _ => None,
            },
            TYPE_GENERIC if self.generic_flags & 1 != 0 => Some(Slot::Shield),
            _ => None,
        }
    }

    /// The magick weight adjustment as the carrier feels it (0x4614a0, 0x461590), for a carrier
    /// with magick/tech `aptitude`.
    fn scaled_adjustment(&self, aptitude: i32) -> i32 {
        let (c, x) = (self.complexity, self.magic_weight_adj);
        let m = (aptitude + c) / 2;
        let a = if c >= 0 {
            m.clamp(0, c)
        } else if m > 0 {
            0
        } else if m < c {
            c
        } else {
            m
        };
        match c {
            0 => x,
            c if c > 0 => x * a / c,
            c => x - x * a / c,
        }
    }

    /// The weight of `quantity` of it carried by a critter with `aptitude` (0x461370).
    pub fn weight(&self, quantity: i32, aptitude: i32) -> i32 {
        if self.kind == TYPE_GOLD {
            return 0;
        }
        let mut w = self.weight;
        if self.magic_weight_adj != 0 {
            w += self.scaled_adjustment(aptitude);
        }
        if self.kind == TYPE_AMMO { w * quantity / 4 } else { w }
    }
}

/// Why an item was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertRefusal {
    /// It would take the total weight past the carry weight.
    TooHeavy,
    /// No free rectangle fits it.
    NoRoom,
}

impl Inventory {
    /// The grid's taken cells (0x466260).
    pub fn occupied(&self, info: impl Fn(u32) -> Option<ItemInfo>) -> Vec<bool> {
        let mut cells = vec![false; (COLUMNS * PC_ROWS) as usize];
        for item in self.items.iter().filter(|i| in_grid(i.location)) {
            let (w, h) = info(item.prototype).map_or((1, 1), |i| i.cells);
            let (col, row) = (item.location % COLUMNS, item.location / COLUMNS);
            for y in row..(row + h).min(PC_ROWS) {
                for x in col..(col + w).min(COLUMNS) {
                    cells[(y * COLUMNS + x) as usize] = true;
                }
            }
        }
        cells
    }

    /// The first free rectangle of `w` x `h` cells, rows first (0x4675a0).
    pub fn free_spot(occupied: &[bool], (w, h): (i32, i32)) -> Option<i32> {
        for row in 0..=PC_ROWS - h {
            for col in 0..=COLUMNS - w {
                let free = (row..row + h).all(|y| (col..col + w).all(|x| !occupied[(y * COLUMNS + x) as usize]));
                if free {
                    return Some(row * COLUMNS + col);
                }
            }
        }
        None
    }

    /// The total weight (0x461410), every item counted, worn ones too.
    pub fn total_weight(&self, info: impl Fn(u32) -> Option<ItemInfo>, aptitude: i32) -> i32 {
        self.items.iter().filter_map(|i| Some(info(i.prototype)?.weight(i.quantity, aptitude))).sum()
    }

    /// Adds `quantity` of `prototype` where the original would (0x461810 with no location):
    /// onto a stack of the same gold or ammo, else the first free spot. `carry` is the carry
    /// weight. Returns the location.
    pub fn insert(&mut self, prototype: u32, quantity: i32, info: impl Fn(u32) -> Option<ItemInfo>, carry: i32, aptitude: i32) -> Result<i32, InsertRefusal> {
        let Some(item) = info(prototype) else { return Err(InsertRefusal::NoRoom) };
        if self.total_weight(&info, aptitude) + item.weight(quantity, aptitude) > carry {
            return Err(InsertRefusal::TooHeavy);
        }
        if item.stacks()
            && let Some(stack) = self.items.iter_mut().find(|i| i.prototype == prototype)
        {
            stack.quantity += quantity;
            return Ok(stack.location);
        }
        let location = Self::free_spot(&self.occupied(&info), item.cells).ok_or(InsertRefusal::NoRoom)?;
        self.items.push(CarriedItem { prototype, location, quantity: if item.stacks() { quantity } else { 0 } });
        Ok(location)
    }

    /// The item in `slot`, if any.
    pub fn worn(&self, slot: Slot) -> Option<&CarriedItem> {
        self.items.iter().find(|i| i.location == slot as i32)
    }

    /// Puts on what fits each empty slot, in slot order (0x4654f0): the first grid item that
    /// goes there. (The original scores the candidates, 0x465170; with one each it is the same.)
    pub fn wield_best(&mut self, info: impl Fn(u32) -> Option<ItemInfo>) {
        for location in FIRST_SLOT..=LAST_SLOT {
            if self.items.iter().any(|i| i.location == location) {
                continue;
            }
            if let Some(item) = self.items.iter_mut().find(|i| in_grid(i.location) && info(i.prototype).and_then(|x| x.slot()).is_some_and(|s| s as i32 == location)) {
                item.location = location;
            }
        }
    }
}

/// A basic prototype number (the one `InvenSource.mes`, backgrounds and descriptions use) is the
/// object with this number added (0x468860): basic 6029, the dagger, is `proto/006049`.
pub const BASIC_TO_PROTOTYPE: u32 = 20;
/// The gold prototype (basic 9056, 0x464970).
pub const GOLD: u32 = 9056 + BASIC_TO_PROTOTYPE;
/// What a created character gets after its background's kit (0x460ff0), as basic numbers: crude
/// lockpicks and bandages.
pub const CREATED_EXTRAS: [u32; 2] = [15178, 15179];
/// Body size by race (0x5b32c0): 1 small, 2 medium, 4 large.
pub const RACE_SIZE: [u8; 8] = [2, 1, 2, 2, 1, 1, 2, 4];

/// The clothes a created character gets when nothing it has is body armour (0x460ff0), as a basic
/// number: by gender and body size, a nice suit for men, a plain dress for women.
pub fn default_clothes(gender: u8, race: u8) -> u32 {
    let size = RACE_SIZE.get(race as usize).copied().unwrap_or(2);
    match (gender == arcanum_rules::FEMALE, size) {
        (true, 1) => 8134,
        (true, 4) => 8135,
        (true, _) => 8133,
        (false, 1) => 8065,
        (false, 4) => 8069,
        (false, _) => 8049,
    }
}

/// A new PC's inventory from its blueprint: its gold, then its items in order (basic numbers),
/// each where the original's insert puts it. A pre-made PC then wields what fits (0x545b60 calls
/// 0x4654f0 after `critter_inventory_source`, 0x463e20). A created one gets default clothes if
/// none of its items is body armour, then [`CREATED_EXTRAS`] (0x460ff0). Items that do not fit
/// are left out, as the original destroys them.
pub fn starting_kit(blueprint: &arcanum_rules::CharacterBlueprint, info: impl Fn(u32) -> Option<ItemInfo>, carry: i32, aptitude: i32) -> Inventory {
    let mut inventory = Inventory::default();
    let add = |inventory: &mut Inventory, basic: u32, quantity: i32| {
        let _ = inventory.insert(basic + BASIC_TO_PROTOTYPE, quantity, &info, carry, aptitude);
    };
    if blueprint.gold > 0 {
        add(&mut inventory, GOLD - BASIC_TO_PROTOTYPE, blueprint.gold);
    }
    for &item in &blueprint.items {
        add(&mut inventory, item, 1);
    }
    if blueprint.prototype.is_some() {
        inventory.wield_best(&info);
        return inventory;
    }
    let dressed = inventory.items.iter().any(|i| info(i.prototype).and_then(|x| x.slot()) == Some(Slot::Armor));
    if !dressed {
        add(&mut inventory, default_clothes(blueprint.sheet.gender, blueprint.sheet.race), 1);
    }
    for extra in CREATED_EXTRAS {
        add(&mut inventory, extra, 1);
    }
    inventory
}

/// The encumbrance level, 0-6, of `total` weight against `carry` (0x45f790).
pub fn encumbrance(total: i32, carry: i32) -> usize {
    ENCUMBRANCE_PERCENT.iter().position(|&p| total < p * carry / 100).unwrap_or(6).min(6)
}

/// Item facts by prototype, from the prototypes and an art size reader.
pub fn item_info(prototypes: &Prototypes, art_size: impl Fn(ArtId) -> Option<(u32, u32)>) -> impl Fn(u32) -> Option<ItemInfo> {
    move |number| prototypes.get(number).map(|p| ItemInfo::of(p, &art_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(kind: u32, cells: (i32, i32), weight: i32) -> ItemInfo {
        ItemInfo { kind, inventory_art: 0, cells, weight, magic_weight_adj: 0, complexity: 0, generic_flags: 0 }
    }

    fn table(n: u32) -> Option<ItemInfo> {
        match n {
            1 => Some(info(TYPE_WEAPON, (2, 4), 50)),
            2 => Some(info(TYPE_ARMOR, (4, 5), 200)),
            3 => Some(info(TYPE_GOLD, (1, 1), 1)),
            4 => Some(info(TYPE_GENERIC, (1, 1), 1)),
            _ => None,
        }
    }

    /// The test table by object number: basic numbers 1-4 are objects 21-24.
    fn objects(n: u32) -> Option<ItemInfo> {
        match n {
            GOLD => table(3),
            n if n == default_clothes(1, 0) + BASIC_TO_PROTOTYPE => table(2),
            n if CREATED_EXTRAS.iter().any(|&e| e + BASIC_TO_PROTOTYPE == n) => table(4),
            n => table(n.checked_sub(BASIC_TO_PROTOTYPE)?),
        }
    }

    #[test]
    fn a_premade_starts_with_its_gold_first_and_wields_its_kit() {
        let blueprint = arcanum_rules::CharacterBlueprint { sheet: Default::default(), gold: 385, items: vec![1, 2, 4, 99], prototype: Some(16094) };
        let kit = starting_kit(&blueprint, objects, 10_000, 0);
        assert_eq!(kit.items[0], CarriedItem { prototype: GOLD, location: 0, quantity: 385 });
        assert_eq!(kit.worn(Slot::Weapon).map(|i| i.prototype), Some(21), "basic 1 is object 21");
        assert_eq!(kit.items.len(), 4, "an unknown prototype is left out");
    }

    #[test]
    fn a_created_pc_gets_clothes_lockpicks_and_bandages() {
        let mut blueprint = arcanum_rules::CharacterBlueprint { sheet: Default::default(), gold: 0, items: vec![1], prototype: None };
        blueprint.sheet.gender = 1;
        let kit = starting_kit(&blueprint, objects, 10_000, 0);
        let numbers: Vec<u32> = kit.items.iter().map(|i| i.prototype - BASIC_TO_PROTOTYPE).collect();
        assert_eq!(numbers, [1, 8049, 15178, 15179], "a medium man's nice suit");
        assert!(kit.items.iter().all(|i| in_grid(i.location)), "nothing worn");
        blueprint.items.push(2);
        let dressed = starting_kit(&blueprint, objects, 10_000, 0);
        assert_eq!(dressed.items.len(), 4, "already has body armour: no suit");
        assert_eq!((default_clothes(0, 0), default_clothes(1, 1), default_clothes(1, 7)), (8133, 8065, 8069));
    }

    #[test]
    fn cells_round_up_to_whole_cells() {
        assert_eq!(cells_for(32, 33), (1, 2));
        assert_eq!(cells_for(64, 128), (2, 4));
        assert_eq!(cells_for(0, 1), (1, 1));
    }

    #[test]
    fn items_go_to_the_first_free_rectangle_rows_first() {
        let mut inv = Inventory::default();
        assert_eq!(inv.insert(1, 0, table, 10_000, 0), Ok(0));
        assert_eq!(inv.insert(2, 0, table, 10_000, 0), Ok(2), "beside the sword");
        assert_eq!(inv.insert(4, 0, table, 10_000, 0), Ok(6));
        assert_eq!(inv.insert(3, 100, table, 10_000, 0), Ok(7));
        assert_eq!(inv.insert(3, 25, table, 10_000, 0), Ok(7), "gold joins its stack");
        assert_eq!(inv.items.iter().find(|i| i.prototype == 3).unwrap().quantity, 125);
    }

    #[test]
    fn a_full_grid_or_a_heavy_load_refuses() {
        let mut inv = Inventory::default();
        assert_eq!(inv.insert(2, 0, table, 150, 0), Err(InsertRefusal::TooHeavy));
        for _ in 0..4 {
            inv.insert(2, 0, table, 10_000, 0).unwrap();
        }
        assert_eq!(inv.insert(2, 0, table, 10_000, 0), Err(InsertRefusal::NoRoom), "four 4x5 leave two columns and two rows");
        assert_eq!(inv.insert(1, 0, table, 10_000, 0), Ok(8), "a 2x4 still fits the last columns");
    }

    #[test]
    fn wielding_puts_items_in_their_slots() {
        let mut inv = Inventory::default();
        inv.insert(1, 0, table, 10_000, 0).unwrap();
        inv.insert(2, 0, table, 10_000, 0).unwrap();
        inv.insert(4, 0, table, 10_000, 0).unwrap();
        inv.wield_best(table);
        assert_eq!(inv.worn(Slot::Weapon).map(|i| i.prototype), Some(1));
        assert_eq!(inv.worn(Slot::Armor).map(|i| i.prototype), Some(2), "coverage 0 is body armour");
        assert_eq!(inv.items[2].location, 6, "a plain generic stays in the grid");
    }

    #[test]
    fn weight_and_encumbrance_follow_the_original() {
        let mut ammo = info(TYPE_AMMO, (1, 1), 1);
        assert_eq!(ammo.weight(50, 0), 12, "a quarter of the count");
        ammo.kind = TYPE_GOLD;
        assert_eq!(ammo.weight(50, 0), 0);
        let mut magic = info(TYPE_WEAPON, (1, 1), 10);
        (magic.magic_weight_adj, magic.complexity) = (-8, 40);
        assert_eq!(magic.weight(0, 40), 2, "a full magick match takes the whole adjustment");
        assert_eq!(magic.weight(0, -40), 10, "a technologist gets none of it");
        assert_eq!((encumbrance(0, 4000), encumbrance(599, 4000), encumbrance(600, 4000), encumbrance(3600, 4000)), (0, 0, 1, 6));
    }
}
