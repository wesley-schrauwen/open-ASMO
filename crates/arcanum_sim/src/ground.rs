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
//! Not built: the pile the original makes when an item is dropped onto a tile that already holds
//! items (container 3023, 0x466ef0) and the loot window it needs; here they lie side by side.
//! The quantity prompt for part of a stack; the decay timer (0x468090); script hooks; the
//! sounds; picking up in combat (an action point); an item's own changed fields (a picked-up
//! map item becomes its prototype).

use arcanum_formats::map::{SectorId, TileLoc};
use arcanum_formats::object::{Object, TYPE_AMMO, TYPE_GENERIC, TYPE_GOLD, TYPE_WEAPON};
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
}

impl GroundItemId {
    pub fn sector(self) -> SectorId {
        match self {
            Self::Mobile { sector, .. } | Self::Dropped { sector, .. } => sector,
        }
    }
}

/// A dropped item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundItem {
    pub prototype: u32,
    /// Gold and ammo: how many.
    pub quantity: i32,
    pub location: TileLoc,
}

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
                Some(GroundItem { prototype: object.prototype.prototype()?, quantity: self.quantity_of(object), location: crate::object_location(object)? })
            }
            GroundItemId::Dropped { number, .. } => changes?.dropped.get(&number).copied(),
        }
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
fn broadcast(session: &Session, outbox: &mut Outbox, delta: LedgerDelta) {
    for (&player, state) in &session.players {
        if matches!(state, SessionPlayer::InWorld(_)) {
            outbox.0.push((player, delta.clone()));
        }
    }
}

/// Mirrors a ground change into the save.
fn save_change(world: &mut WorldState, map: &LoadedMap, sector: SectorId) {
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
    let changed = inventory.apply(&InventoryRequest::Drop { from }, Wearer { race: 0, gender: 0 }, info)?;
    let Some(save) = world.0.as_mut() else { return Err(ItemRefusal::Silent) };
    let number = save.next_dropped;
    save.next_dropped += 1;
    let id = GroundItemId::Dropped { sector: at.sector(), number };
    let ground = GroundItem { prototype: item.prototype, quantity: item.quantity, location: at };
    map.drop_ground_item(id, ground);
    save_change(world, map, id.sector());
    broadcast(session, outbox, LedgerDelta::GroundItemDropped { item: id, ground });
    Ok(changed)
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
        let Some(ground) = map.ground_item(want.item) else {
            commands.entity(entity).remove::<PendingPickUp>();
            continue;
        };
        if !within_radius(position.0, ground.location, crate::USE_RANGE) {
            if walking.is_some() {
                commands.entity(entity).insert(want);
            } else {
                commands.entity(entity).remove::<PendingPickUp>();
            }
            continue;
        }
        commands.entity(entity).remove::<PendingPickUp>();
        let player = controlled.0;
        let derived = rules.0.derived(&sheet.0);
        let aptitude = rules.0.aptitude(&sheet.0);
        let (prototypes, art) = (map.prototypes.clone(), map.art.clone());
        let info = crate::item_info(&prototypes, |id| crate::world::art_size(vfs, art.as_deref(), id));
        let mut changed = inventory.clone();
        match changed.insert(ground.prototype, ground.quantity.max(1), &info, derived.carry_weight, aptitude) {
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
