//! Keys and key rings (Arcanum.exe 0x463370 / 0x463240 / 0x463340, the ring 0x466a00 /
//! 0x466a50 / 0x466aa0).
//!
//! - **A key** (object type 11) opens the locks whose key number (`portal_key_id`,
//!   `container_key_id`) matches its `key_key_id` (0x463340): lock 0 has no key; lock 1 takes
//!   any key; key 1 is a master key opening every keyed lock; otherwise the numbers must be
//!   equal. Each carried key keeps its own number ([`crate::CarriedItem::keys`]).
//! - **A key ring** (type 12) holds key numbers (`key_ring_list_idx`). A key put into a critter
//!   that carries a ring goes onto the ring and is gone as an item: it needs no room and adds no
//!   weight (0x466510 / 0x466a00). A ring put into a critter takes in its loose keys, or gives its
//!   numbers to the ring it already has (0x466aa0). The ring used is the first with keys on it,
//!   else the last (0x4631a0). With no ring, keys are loose items.
//! - **Who has the key** (0x463370): the critter's own inventory, its rings included; the
//!   original also searches the party's followers (none yet).
//! - Opening with a key does not unlock: the lock stays and is checked again next time.
//!
//! Not built: the followers' keys, the ring's art showing empty or full (0x466bd0), the key ring
//! window.

use arcanum_formats::object::{Object, Prototypes, TYPE_KEY, TYPE_KEY_RING};

use crate::{Inventory, ItemInfo, PC_ROWS};

/// Whether key number `key` opens lock number `lock` (0x463340).
pub fn key_matches(lock: i32, key: i32) -> bool {
    if lock == 0 || key == 0 {
        return false;
    }
    lock == 1 || key == 1 || lock == key
}

/// The key numbers an item object carries: a key's own, or a ring's list.
pub fn object_keys(object: &Object, prototypes: &Prototypes) -> Vec<i32> {
    match object.object_type {
        TYPE_KEY => prototypes.int(object, "key_key_id").filter(|&k| k != 0).into_iter().collect(),
        TYPE_KEY_RING => object.ints("key_ring_list_idx").or_else(|| prototypes.of(object)?.ints("key_ring_list_idx")).unwrap_or_default(),
        _ => Vec::new(),
    }
}

impl Inventory {
    /// Whether a key or ring carried opens lock number `lock` (0x463240).
    pub fn has_key(&self, lock: i32) -> bool {
        self.items.iter().flat_map(|i| &i.keys).any(|&k| key_matches(lock, k))
    }

    /// The ring keys go onto: the first with keys on it, else the last (0x4631a0).
    pub(crate) fn ring(&self, info: &impl Fn(u32) -> Option<ItemInfo>) -> Option<usize> {
        let rings: Vec<usize> = (0..self.items.len()).filter(|&i| info(self.items[i].prototype).is_some_and(|f| f.kind == TYPE_KEY_RING)).collect();
        rings.iter().copied().find(|&i| !self.items[i].keys.is_empty()).or(rings.last().copied())
    }

    /// A critter's ring takes in its loose keys and the other rings' numbers (0x466a00 /
    /// 0x466aa0). Containers keep keys as they are.
    pub fn fold_keys(&mut self, info: &impl Fn(u32) -> Option<ItemInfo>) {
        if self.rows != PC_ROWS {
            return;
        }
        let Some(ring) = self.ring(info) else { return };
        let mut taken = Vec::new();
        for i in 0..self.items.len() {
            if i == ring {
                continue;
            }
            let kind = info(self.items[i].prototype).map(|f| f.kind);
            if matches!(kind, Some(TYPE_KEY | TYPE_KEY_RING)) {
                taken.append(&mut self.items[i].keys);
            }
        }
        self.items[ring].keys.append(&mut taken);
        let items = std::mem::take(&mut self.items);
        self.items = items.into_iter().filter(|i| info(i.prototype).is_none_or(|f| f.kind != TYPE_KEY)).collect();
    }

    /// Whether a key put in here goes onto a ring rather than into the grid.
    pub fn takes_onto_ring(&self, prototype: u32, info: &impl Fn(u32) -> Option<ItemInfo>) -> bool {
        self.rows == PC_ROWS && info(prototype).is_some_and(|f| f.kind == TYPE_KEY) && self.ring(info).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CarriedItem;

    #[test]
    fn key_numbers_match_as_the_original() {
        assert!(key_matches(14, 14));
        assert!(!key_matches(14, 15));
        assert!(!key_matches(0, 14), "a lock with no key");
        assert!(!key_matches(14, 0));
        assert!(!key_matches(0, 1), "not even a master key");
        assert!(key_matches(1, 23), "any key");
        assert!(key_matches(23, 1), "a master key");
    }

    fn info(n: u32) -> Option<ItemInfo> {
        let kind = match n {
            1 => TYPE_KEY,
            2 => TYPE_KEY_RING,
            _ => arcanum_formats::object::TYPE_GENERIC,
        };
        Some(ItemInfo { kind, inventory_art: 0, cells: (1, 1), weight: 1, magic_weight_adj: 0, complexity: 0, generic_flags: 0, weapon_flags: 0, armor_flags: 0, item_flags: 0, worth: 0 })
    }

    fn key(location: i32, keys: Vec<i32>) -> CarriedItem {
        CarriedItem { prototype: 1, location, quantity: 0, keys }
    }

    #[test]
    fn keys_go_onto_the_ring() {
        let mut inv = Inventory { items: vec![key(0, vec![14]), CarriedItem { prototype: 3, location: 1, quantity: 0, keys: vec![] }, key(2, vec![23])], ..Default::default() };
        inv.fold_keys(&info);
        assert_eq!(inv.items.len(), 3, "no ring: loose keys stay");
        assert!(inv.has_key(23));
        inv.items.push(CarriedItem { prototype: 2, location: 5, quantity: 0, keys: vec![7] });
        inv.fold_keys(&info);
        assert_eq!(inv.items.iter().map(|i| (i.prototype, i.keys.clone())).collect::<Vec<_>>(), [(3, vec![]), (2, vec![7, 14, 23])]);
        assert!(inv.has_key(14) && inv.has_key(7) && !inv.has_key(8));
        assert!(inv.takes_onto_ring(1, &info));
        let chest = Inventory { items: vec![key(0, vec![14]), CarriedItem { prototype: 2, location: 1, quantity: 0, keys: vec![] }], ..Inventory::container() };
        let mut folded = chest.clone();
        folded.fold_keys(&info);
        assert_eq!(folded, chest, "a container keeps its keys loose");
    }

    #[test]
    fn a_second_ring_gives_its_numbers_to_the_first() {
        let ring = |location, keys| CarriedItem { prototype: 2, location, quantity: 0, keys };
        let mut inv = Inventory { items: vec![ring(0, vec![]), ring(1, vec![5]), ring(2, vec![6])], ..Default::default() };
        inv.fold_keys(&info);
        let keys: Vec<Vec<i32>> = inv.items.iter().map(|i| i.keys.clone()).collect();
        assert_eq!(keys, [vec![], vec![5, 6], vec![]], "the first with keys takes them; the others stay, empty");
    }
}
