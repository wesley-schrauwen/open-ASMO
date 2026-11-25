//! Items on the ground: dropping from the inventory and picking up off the map (Arcanum.exe
//! drop 0x461ab0 / place 0x4415c0; take 0x42aa70 then insert 0x4617f0).
//!
//! - **Dropping** (the inventory screen's drop box): the item leaves the inventory and lands on
//!   the PC's own tile (radius 0, 0x461a70), whole stacks included. An item with `item_flags`
//!   `NO_DROP` (0x20) is refused ("The item cannot be dropped.", `mes/item.mes` 111).
//! - **Picking up:** clicking an item on the map walks the PC to within [`crate::USE_RANGE`],
//!   then it goes into the inventory where the insert puts it (gold and ammo onto their stack).
//!   Too heavy or no room leaves it on the ground with the refusal (101, 102).
//! - The map's own items (`.mob` files) are named by their place in the sector's mobile objects;
//!   dropped ones by a number the host gives. Both are kept in the save's sector changes and
//!   sent to every player in the world.
//!
//! - **Piles** ([`Pile`], 0x466ef0): an item dropped onto a tile with a pile goes into the first
//!   with room; onto a tile with bare items, a new junk pile (basic 3023) takes them, then the
//!   dropped item. Clicking a pile walks the PC to it and opens the loot window
//!   ([`LedgerDelta::PileOpened`]): [`InventoryRequest::Take`], `Put` and `TakeAll` (what fits,
//!   in order, refusals silent: 0x579770). On close (0x463540), once nobody else has it open, an
//!   empty pile goes and a pile of one item leaves that item bare on its tile.
//!
//! The map's own containers open into the same loot window ([`crate::container`]).
//!
//! Not built: moving items within a pile. The quantity prompt for part of a stack; script hooks; the
//! sounds; picking up in combat (an action point); an item's own changed fields (a picked-up
//! map item becomes its prototype). Dropped items never decay: the original's timer (0x468090)
//! is left out by the owner's decision.

use arcanum_formats::map::{SectorId, TileLoc};
use arcanum_formats::object::{Object, TYPE_AMMO, TYPE_CONTAINER, TYPE_GENERIC, TYPE_GOLD, TYPE_WEAPON, Value};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Inventory, InventoryRequest, ItemRefusal, LedgerDelta, LoadedMap, NetId, Outbox, PlayerControlled, PlayerId, Position, Session, SessionPlayer, Sheet, SimRules, WorldState, Wearer, within_radius};

/// An item lying on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GroundItemId {
    /// One of the map's own items: its place in the sector's mobile objects.
    Mobile { sector: SectorId, index: u32 },
    /// A dropped item, by the host's number.
    Dropped { sector: SectorId, number: u32 },
    /// A pile, by the host's number (the same count as dropped items).
    Pile { sector: SectorId, number: u32 },
    /// One of the map's containers ([`crate::container`]): its place in the sector's mobile
    /// objects.
    Container { sector: SectorId, index: u32 },
}

impl GroundItemId {
    pub fn sector(self) -> SectorId {
        match self {
            Self::Mobile { sector, .. } | Self::Dropped { sector, .. } | Self::Pile { sector, .. } | Self::Container { sector, .. } => sector,
        }
    }
}

/// A dropped item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundItem {
    pub prototype: u32,
    /// Gold and ammo: how many.
    pub quantity: i32,
    pub location: TileLoc,
    /// A key's number, or a key ring's ([`crate::keys`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<i32>,
    /// A written item's first line ([`crate::CarriedItem::written`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub written: i32,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

impl GroundItem {
    /// The item as carried, at `location`.
    pub fn carried(&self, location: i32) -> crate::CarriedItem {
        crate::CarriedItem { prototype: self.prototype, location, quantity: self.quantity, keys: self.keys.clone(), written: self.written }
    }
}

/// A junk pile: the container the original makes when an item is dropped onto items (basic
/// prototype 3023, "Junk Pile", 0x466ef0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pile {
    pub location: TileLoc,
    pub inventory: Inventory,
}

/// The junk pile's prototype (basic 3023).
pub const PILE: u32 = 3023 + crate::BASIC_TO_PROTOTYPE;

/// Whether objects of `kind` are items (weapons to generic, 5-14).
pub fn is_item(kind: u32) -> bool {
    (TYPE_WEAPON..=TYPE_GENERIC).contains(&kind)
}

impl LoadedMap {
    /// The item `id`, if it lies on the ground in a sector this map has read.
    pub fn ground_item(&self, id: GroundItemId) -> Option<GroundItem> {
        let changes = self.changes.get(&id.sector());
        match id {
            GroundItemId::Mobile { sector, index } => {
                if changes.is_some_and(|c| c.taken.contains(&index)) {
                    return None;
                }
                let object = self.mobiles.get(&sector)?.get(index as usize).filter(|o| is_item(o.object_type))?;
                Some(GroundItem { prototype: object.prototype.prototype()?, quantity: self.quantity_of(object), location: crate::object_location(object)?, keys: crate::object_keys(object, &self.prototypes), written: crate::object_written(object, &self.prototypes) })
            }
            GroundItemId::Dropped { number, .. } => changes?.dropped.get(&number).cloned(),
            GroundItemId::Pile { .. } | GroundItemId::Container { .. } => None,
        }
    }

    /// The loot window's other side for `id`: a pile, or one of the map's containers with its
    /// contents (its changed ones, else the map's stored list).
    pub fn pile(&self, id: GroundItemId) -> Option<Pile> {
        match id {
            GroundItemId::Pile { sector, number } => self.changes.get(&sector)?.piles.get(&number).cloned(),
            GroundItemId::Container { sector, index } => {
                let object = self.container_object(id)?;
                let location = crate::object_location(object)?;
                if let Some(inventory) = self.changes.get(&sector).and_then(|c| c.containers.get(&index)) {
                    return Some(Pile { location, inventory: inventory.clone() });
                }
                Some(Pile { location, inventory: self.stored_contents(object) })
            }
            _ => None,
        }
    }

    /// The map's container `id`.
    pub fn container_object(&self, id: GroundItemId) -> Option<&Object> {
        let GroundItemId::Container { sector, index } = id else { return None };
        self.mobiles.get(&sector)?.get(index as usize).filter(|o| o.object_type == TYPE_CONTAINER)
    }

    /// A container's stored list (`container_inventory_list_idx`), each item at its stored
    /// `item_inv_location`.
    pub fn stored_contents(&self, container: &Object) -> Inventory {
        let mut contents = Inventory::container();
        contents.items = self.listed_items(container, "container_inventory_list_idx");
        contents
    }

    /// A critter's stored list (`critter_inventory_list_idx`), each item at its stored
    /// `item_inv_location` (worn ones in their slots, 1000-1008).
    pub fn critter_contents(&self, critter: &Object) -> Inventory {
        Inventory { items: self.listed_items(critter, "critter_inventory_list_idx"), ..Inventory::default() }
    }

    fn listed_items(&self, owner: &Object, field: &str) -> Vec<crate::CarriedItem> {
        let ids = match owner.field(field) {
            Some(Value::Array(Some(list))) => list.object_ids().unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut items = Vec::new();
        for id in ids {
            let Some(item) = self.stored_items.get(&id) else { continue };
            let Some(prototype) = item.prototype.prototype() else { continue };
            let location = self.prototypes.int(item, "item_inv_location").unwrap_or(0);
            items.push(crate::CarriedItem { prototype, location, quantity: self.quantity_of(item), keys: crate::object_keys(item, &self.prototypes), written: crate::object_written(item, &self.prototypes) });
        }
        items
    }

    /// The item prototypes of a critter's stored list whose items are flagged `item_flags`
    /// 0x2000 (kept when it restocks).
    pub fn kept_kinds(&self, critter: &Object) -> std::collections::BTreeSet<u32> {
        let ids = match critter.field("critter_inventory_list_idx") {
            Some(Value::Array(Some(list))) => list.object_ids().unwrap_or_default(),
            _ => Vec::new(),
        };
        ids.iter()
            .filter_map(|id| self.stored_items.get(id))
            .filter(|item| self.prototypes.int(item, "item_flags").unwrap_or(0) & 0x2000 != 0)
            .filter_map(|item| item.prototype.prototype())
            .collect()
    }

    /// NPC `index` of `sector`'s mobile objects' inventory: as changed (bartered), else as stored.
    pub fn npc_inventory(&self, sector: SectorId, index: u32) -> Option<Inventory> {
        if let Some(changed) = self.changes.get(&sector).and_then(|c| c.npc_inventories.get(&index)) {
            return Some(changed.clone());
        }
        let npc = self.mobiles.get(&sector)?.get(index as usize)?;
        Some(self.critter_contents(npc))
    }

    /// Keeps NPC `index`'s changed inventory (saved with the sector).
    pub fn set_npc_inventory(&mut self, sector: SectorId, index: u32, inventory: Inventory) {
        self.changes.entry(sector).or_default().npc_inventories.insert(index, inventory);
    }

    /// Sets pile `id`'s contents, or removes it; a container keeps its changed contents.
    pub fn set_pile(&mut self, id: GroundItemId, pile: Option<Pile>) {
        match id {
            GroundItemId::Pile { sector, number } => {
                let changes = self.changes.entry(sector).or_default();
                match pile {
                    Some(p) => {
                        changes.piles.insert(number, p);
                    }
                    None => {
                        changes.piles.remove(&number);
                    }
                }
            }
            GroundItemId::Container { sector, index } => {
                if let Some(p) = pile {
                    self.changes.entry(sector).or_default().containers.insert(index, p.inventory);
                }
            }
            _ => {}
        }
    }

    /// Whether container `id` shows open (frame 1).
    pub fn container_open(&self, id: GroundItemId) -> bool {
        let GroundItemId::Container { sector, index } = id else { return false };
        self.changes.get(&sector).is_some_and(|c| c.opened.contains(&index))
    }

    /// Shows container `id` open or shut.
    pub fn set_container_open(&mut self, id: GroundItemId, open: bool) {
        let GroundItemId::Container { sector, index } = id else { return };
        let changes = self.changes.entry(sector).or_default();
        if open {
            changes.opened.insert(index);
        } else {
            changes.opened.remove(&index);
        }
    }

    /// The bare items lying on tile `at`: the map's own, in file order, then dropped ones.
    pub fn items_on_tile(&self, at: TileLoc) -> Vec<(GroundItemId, GroundItem)> {
        let sector = at.sector();
        let mobiles = self.mobiles.get(&sector).map_or(0, Vec::len) as u32;
        let own = (0..mobiles).map(|index| GroundItemId::Mobile { sector, index });
        let dropped = self.changes.get(&sector).into_iter().flat_map(|c| c.dropped.keys().map(|&number| GroundItemId::Dropped { sector, number }));
        own.chain(dropped).filter_map(|id| Some((id, self.ground_item(id)?))).filter(|(_, g)| g.location == at).collect()
    }

    /// The piles on tile `at`, oldest first.
    pub fn piles_on_tile(&self, at: TileLoc) -> Vec<GroundItemId> {
        let sector = at.sector();
        self.changes.get(&sector).into_iter().flat_map(|c| c.piles.iter().filter(|(_, p)| p.location == at).map(|(&number, _)| GroundItemId::Pile { sector, number })).collect()
    }

    /// A stack's quantity (gold and ammo), its own or its prototype's.
    fn quantity_of(&self, object: &Object) -> i32 {
        match object.object_type {
            TYPE_GOLD => self.prototypes.int(object, "gold_quantity").unwrap_or(1),
            TYPE_AMMO => self.prototypes.int(object, "ammo_quantity").unwrap_or(1),
            _ => 0,
        }
    }

    /// Takes item `id` off the ground.
    pub fn take_ground_item(&mut self, id: GroundItemId) {
        let changes = self.changes.entry(id.sector()).or_default();
        match id {
            GroundItemId::Mobile { index, .. } => {
                changes.taken.insert(index);
            }
            GroundItemId::Dropped { number, .. } => {
                changes.dropped.remove(&number);
            }
            GroundItemId::Pile { number, .. } => {
                changes.piles.remove(&number);
            }
            GroundItemId::Container { .. } => {}
        }
    }

    /// Puts a dropped item on the ground.
    pub fn drop_ground_item(&mut self, id: GroundItemId, item: GroundItem) {
        if let GroundItemId::Dropped { number, .. } = id {
            self.changes.entry(id.sector()).or_default().dropped.insert(number, item);
        }
    }
}

/// A player's request to pick an item up, waiting for their PC to be in range. `seq` is the
/// move it came with.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingPickUp {
    pub item: GroundItemId,
    pub seq: u32,
}

/// Pick-up requests taken from the inbox.
#[derive(Resource, Debug, Default)]
pub struct PickUpRequests(pub Vec<(PlayerId, GroundItemId, u32)>);

/// Each player's last inventory request answered, so a pick-up's answer acknowledges it too.
#[derive(Resource, Debug, Default)]
pub struct InventoryAcks(pub std::collections::HashMap<PlayerId, u32>);

/// Tells every player in the world.
pub(crate) fn broadcast(session: &Session, outbox: &mut Outbox, delta: LedgerDelta) {
    for (&player, state) in &session.players {
        if matches!(state, SessionPlayer::InWorld(_)) {
            outbox.0.push((player, delta.clone()));
        }
    }
}

/// Mirrors a ground change into the save.
pub(crate) fn save_change(world: &mut WorldState, map: &LoadedMap, sector: SectorId) {
    if let Some(save) = world.0.as_mut() {
        match map.changes.get(&sector) {
            Some(delta) => {
                save.sector_changes.insert(sector, delta.clone());
            }
            None => {
                save.sector_changes.remove(&sector);
            }
        }
    }
}

/// Drops the item at inventory location `from` of `player`'s PC on its tile. Returns the
/// changed inventory.
#[allow(clippy::too_many_arguments)]
pub fn drop_item(
    inventory: &Inventory,
    from: i32,
    at: TileLoc,
    info: impl Fn(u32) -> Option<crate::ItemInfo>,
    map: &mut LoadedMap,
    world: &mut WorldState,
    session: &Session,
    outbox: &mut Outbox,
) -> Result<Inventory, ItemRefusal> {
    let item = inventory.items.iter().find(|i| i.location == from).cloned().ok_or(ItemRefusal::Silent)?;
    let changed = inventory.apply(&InventoryRequest::Drop { from }, Wearer { race: 0, gender: 0 }, &info)?;
    place_on_ground(item, at, &info, map, world, session, outbox);
    Ok(changed)
}

/// Puts `item` on tile `at` as the original's drop does (0x466e50, 0x466ef0): into the first pile
/// there with room; else, if bare items lie there, into a new pile with them (they go first);
/// else bare on the tile.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_on_ground(item: crate::CarriedItem, at: TileLoc, info: &impl Fn(u32) -> Option<crate::ItemInfo>, map: &mut LoadedMap, world: &mut WorldState, session: &Session, outbox: &mut Outbox) {
    let sector = at.sector();
    for pile_id in map.piles_on_tile(at) {
        let Some(pile) = map.pile(pile_id) else { continue };
        let mut contents = pile.inventory.clone();
        if contents.insert_item(item.clone(), info, i32::MAX, 0).is_ok() {
            let pile = Pile { location: at, inventory: contents };
            map.set_pile(pile_id, Some(pile.clone()));
            save_change(world, map, sector);
            broadcast(session, outbox, LedgerDelta::PileChanged { pile: pile_id, contents: Some(pile) });
            return;
        }
    }
    let Some(save) = world.0.as_mut() else { return };
    let number = save.next_dropped;
    save.next_dropped += 1;
    let bare = map.items_on_tile(at);
    if bare.is_empty() {
        let id = GroundItemId::Dropped { sector, number };
        let ground = GroundItem { prototype: item.prototype, quantity: item.quantity, location: at, keys: item.keys, written: item.written };
        map.drop_ground_item(id, ground.clone());
        save_change(world, map, sector);
        broadcast(session, outbox, LedgerDelta::GroundItemDropped { item: id, ground });
        return;
    }
    let mut contents = Inventory::container();
    for (id, ground) in bare {
        if contents.insert_item(ground.carried(0), info, i32::MAX, 0).is_ok() {
            map.take_ground_item(id);
            broadcast(session, outbox, LedgerDelta::GroundItemTaken { item: id });
        }
    }
    let _ = contents.insert_item(item, info, i32::MAX, 0);
    let pile_id = GroundItemId::Pile { sector, number };
    let pile = Pile { location: at, inventory: contents };
    map.set_pile(pile_id, Some(pile.clone()));
    save_change(world, map, sector);
    broadcast(session, outbox, LedgerDelta::PileChanged { pile: pile_id, contents: Some(pile) });
}

/// The pile a PC's loot window shows.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LootOpen(pub GroundItemId);

/// Applies a loot request of `player`'s PC on its open pile. Returns the PC's changed inventory.
#[allow(clippy::too_many_arguments)]
pub fn apply_loot(
    request: &InventoryRequest,
    open: Option<GroundItemId>,
    pc: &Inventory,
    wearer: Wearer,
    carry: i32,
    aptitude: i32,
    others_open: bool,
    info: impl Fn(u32) -> Option<crate::ItemInfo>,
    map: &mut LoadedMap,
    world: &mut WorldState,
    session: &Session,
    outbox: &mut Outbox,
) -> Result<Inventory, ItemRefusal> {
    let pile_id = open.ok_or(ItemRefusal::Silent)?;
    let pile = map.pile(pile_id).ok_or(ItemRefusal::Silent)?;
    let (pc, contents) = match *request {
        InventoryRequest::Take { from, to } => {
            let (rest, item) = pile.inventory.take_out(from).ok_or(ItemRefusal::Silent)?;
            (pc.put_in(&item, to, wearer, &info, carry, aptitude)?, rest)
        }
        InventoryRequest::Put { from, to } => {
            let (rest, item) = pc.take_out(from).ok_or(ItemRefusal::Silent)?;
            if info(item.prototype).is_some_and(|i| i.item_flags & crate::ITEM_NO_DROP != 0) {
                return Err(ItemRefusal::NoDrop);
            }
            (rest, pile.inventory.put_in(&item, to, wearer, &info, i32::MAX, 0)?)
        }
        InventoryRequest::TakeAll => {
            // Each item the PC can take, in order; the rest stay, with no message (0x579770).
            let (mut pc, mut contents) = (pc.clone(), pile.inventory.clone());
            for item in pile.inventory.items.clone() {
                if let Ok(taken) = pc.put_in(&item, None, wearer, &info, carry, aptitude) {
                    pc = taken;
                    contents = contents.take_out(item.location).map_or(contents, |(rest, _)| rest);
                }
            }
            (pc, contents)
        }
        InventoryRequest::ClosePile => {
            // A container shuts once nobody has it open, unless sticky (0x573440); it stays,
            // even empty.
            if let Some(object) = map.container_object(pile_id) {
                let sticky = map.prototypes.int(object, "container_flags").unwrap_or(0) & crate::OCOF_STICKY != 0;
                if !others_open && !sticky && map.container_open(pile_id) {
                    map.set_container_open(pile_id, false);
                    save_change(world, map, pile_id.sector());
                    broadcast(session, outbox, LedgerDelta::ContainerShown { container: pile_id, open: false });
                }
                return Ok(pc.clone());
            }
            // Collapses only once nobody has it open (0x463540 runs on close).
            if !others_open {
                let sector = pile_id.sector();
                match pile.inventory.items.as_slice() {
                    [] => {
                        map.set_pile(pile_id, None);
                        save_change(world, map, sector);
                        broadcast(session, outbox, LedgerDelta::PileChanged { pile: pile_id, contents: None });
                    }
                    [only] => {
                        map.set_pile(pile_id, None);
                        broadcast(session, outbox, LedgerDelta::PileChanged { pile: pile_id, contents: None });
                        place_on_ground(only.clone(), pile.location, &info, map, world, session, outbox);
                        save_change(world, map, sector);
                    }
                    _ => {}
                }
            }
            return Ok(pc.clone());
        }
        _ => return Err(ItemRefusal::Silent),
    };
    let pile = Pile { location: pile.location, inventory: contents };
    map.set_pile(pile_id, Some(pile.clone()));
    save_change(world, map, pile_id.sector());
    broadcast(session, outbox, LedgerDelta::PileChanged { pile: pile_id, contents: Some(pile) });
    Ok(pc)
}

type PickerQuery<'a> = (Entity, &'a NetId, &'a PlayerControlled, &'a Position, Option<&'a crate::Walking>, Option<&'a PendingPickUp>, &'a Sheet, &'a mut Inventory);

/// Takes pick-up requests (each becomes the PC's [`PendingPickUp`]), then picks up the items whose
/// PC is in range; a request whose PC stopped out of range is dropped.
#[allow(clippy::too_many_arguments)]
pub fn pick_up_items(
    mut commands: Commands,
    mut requests: ResMut<PickUpRequests>,
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    session: Res<Session>,
    rules: Res<SimRules>,
    acks: Res<InventoryAcks>,
    vfs: Option<Res<crate::SimVfs>>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<PickerQuery>,
    mut scripts: crate::ScriptHost,
) {
    let mut fresh: std::collections::HashMap<Entity, PendingPickUp> = std::collections::HashMap::new();
    for (player, item, seq) in requests.0.drain(..) {
        if let Some((entity, ..)) = pcs.iter().find(|pc| pc.2.0 == player) {
            fresh.insert(entity, PendingPickUp { item, seq });
        }
    }
    let vfs_guard = vfs.as_ref().and_then(|v| v.0.read().ok());
    let vfs = vfs_guard.as_deref();
    for (entity, &net_id, controlled, position, walking, pending, sheet, mut inventory) in &mut pcs {
        let Some(want) = fresh.get(&entity).copied().or(pending.copied()) else { continue };
        let target = map.ground_item(want.item).map(|g| g.location).or(map.pile(want.item).map(|p| p.location));
        let Some(location) = target else {
            commands.entity(entity).remove::<PendingPickUp>();
            continue;
        };
        if !within_radius(position.0, location, crate::USE_RANGE) {
            if walking.is_some() {
                commands.entity(entity).insert(want);
            } else {
                commands.entity(entity).remove::<PendingPickUp>();
            }
            continue;
        }
        commands.entity(entity).remove::<PendingPickUp>();
        let player = controlled.0;
        // A container opens unless locked, jammed or held; its source rolls on the first opening.
        if map.container_object(want.item).is_some() {
            // The container's "unlock" script (0x4aed80: may deny the opening), then its "use"
            // script (0x428c17 / 0x572370): it may handle the opening itself.
            let attachee = crate::ScriptObject::ground(want.item).expect("a container");
            if scripts.hook(&mut world, &mut map, &mut outbox, attachee, crate::EVENT_UNLOCK, player) == crate::HookResult::SkipDefault {
                continue;
            }
            if scripts.hook(&mut world, &mut map, &mut outbox, attachee, crate::EVENT_USE, player) == crate::HookResult::SkipDefault {
                continue;
            }
        }
        if let Some(object) = map.container_object(want.item) {
            let flags = map.prototypes.int(object, "container_flags").unwrap_or(0);
            let lock = map.prototypes.int(object, "container_key_id").unwrap_or(0);
            if let Err(refusal) = crate::open_check(flags, map.lock_day, inventory.has_key(lock)) {
                outbox.0.push((player, LedgerDelta::OpenRefused { message: refusal.message() }));
                continue;
            }
            let first = !map.changes.get(&want.item.sector()).is_some_and(|c| matches!(want.item, GroundItemId::Container { index, .. } if c.containers.contains_key(&index)));
            let turns = map.prototypes.art_id(object).is_some_and(crate::opens_to_frame);
            let source = map.prototypes.int(object, "container_inventory_source").and_then(|s| u32::try_from(s).ok()).filter(|&s| s != 0);
            let Some(mut pile) = map.pile(want.item) else { continue };
            if first {
                if flags & crate::OCOF_INVEN_SPAWN_PENDING != 0
                    && let Some(source) = source.and_then(|s| crate::inventory_source(vfs?, s))
                {
                    let (prototypes, art) = (map.prototypes.clone(), map.art.clone());
                    let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs, art.as_deref(), id));
                    crate::roll_source(&mut pile.inventory, &source, &mut crate::Rng(crate::roll_seed(pile.location)), info);
                }
                map.set_pile(want.item, Some(pile.clone()));
                broadcast(&session, &mut outbox, LedgerDelta::PileChanged { pile: want.item, contents: Some(pile) });
            }
            if turns && !map.container_open(want.item) {
                map.set_container_open(want.item, true);
                broadcast(&session, &mut outbox, LedgerDelta::ContainerShown { container: want.item, open: true });
            }
            save_change(&mut world, &map, want.item.sector());
            commands.entity(entity).insert(LootOpen(want.item));
            outbox.0.push((player, LedgerDelta::PileOpened { pile: want.item }));
            continue;
        }
        // A pile opens the loot window (0x57cb10, mode 2).
        if map.pile(want.item).is_some() {
            commands.entity(entity).insert(LootOpen(want.item));
            outbox.0.push((player, LedgerDelta::PileOpened { pile: want.item }));
            continue;
        }
        let Some(ground) = map.ground_item(want.item) else { continue };
        // The item's "get" script first (0x461810): it may keep the item where it is. A map
        // item has its own attachments; a dropped one its prototype's.
        let get = match crate::ScriptObject::ground(want.item) {
            Some(object) => scripts.hook(&mut world, &mut map, &mut outbox, object, crate::EVENT_GET, player),
            None => scripts.item_hook(&mut world, &mut map, &mut outbox, player, ground.prototype, crate::EVENT_GET),
        };
        if get == crate::HookResult::SkipDefault {
            continue;
        }
        let Some(ground) = map.ground_item(want.item) else { continue };
        let derived = rules.0.derived(&sheet.0);
        let aptitude = rules.0.aptitude(&sheet.0);
        let (prototypes, art) = (map.prototypes.clone(), map.art.clone());
        let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs, art.as_deref(), id));
        let mut changed = inventory.clone();
        match changed.insert_item(ground.carried(0), &info, derived.carry_weight, aptitude) {
            Ok(_) => {
                *inventory = changed;
                map.take_ground_item(want.item);
                save_change(&mut world, &map, want.item.sector());
                if let Some(saved) = world.0.as_mut().and_then(|w| w.players.get_mut(&player)) {
                    saved.inventory = Some(inventory.clone());
                }
                broadcast(&session, &mut outbox, LedgerDelta::GroundItemTaken { item: want.item });
                let ack = acks.0.get(&player).copied().unwrap_or(0);
                outbox.0.push((player, LedgerDelta::Inventory { net_id, ack, inventory: inventory.clone() }));
            }
            Err(refusal) => {
                let message = match refusal {
                    crate::InsertRefusal::TooHeavy => 101,
                    crate::InsertRefusal::NoRoom => 102,
                };
                outbox.0.push((player, LedgerDelta::ItemRefused { message }));
            }
        }
    }
}
