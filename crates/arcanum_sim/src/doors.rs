//! Doors (portals) opening and closing: shared state, decided by the host.
//!
//! As in the original (`ag_use_object` 0x26 → `ag_open_door` / `ag_close_door`, art steps
//! 0x4f0950 / 0x4f0a90):
//! - A player asks with [`PlayerIntent::UsePortal`](crate::PlayerIntent::UsePortal); the PC
//!   must be within [`USE_RANGE`] tiles of the door, so the client walks it there first. The host
//!   keeps the request ([`PendingUse`]) until its PC is in range, or drops it when the PC stops
//!   short or the player moves on.
//! - A closed door opens unless jammed (windows are jammed portals), magically held or locked
//!   ([`locked`], by the game clock's hour) without its key (`portal_key_id`, [`crate::keys`]:
//!   the PC's keys and rings; opening with the key leaves the door locked); an open one closes
//!   unless magically held.
//! - Only the art's frame changes: closed is frame 0; open is frame 1 for a plain door, or the
//!   last of the swing frames (bit 10 set: 4-6 for rotations 0 and 6, else 1-3). Blocking
//!   follows the frame ([`crate::blocking`]).
//! - A door opened without `STICKY` (0x100) closes itself [`AUTO_CLOSE_TICKS`] after opening,
//!   once no critter is within [`USE_RANGE`] of it (`ag_pend_closing_door`, 0x42b7f0).
//! - Every player in the world gets a [`LedgerDelta::PortalChanged`]; the change goes into
//!   the save's sector changes, and a player who joins gets them all
//!   ([`LedgerDelta::MapChanges`]).
//!
//! Not reproduced: lockpicking, the followers' keys, paths through a locked door the PC has the
//! key for (the original's pathing asks 0x4aeb70 per critter; here locked doors block every
//! path, and the PC walks up to one and opens it), scripts that veto a use, the door
//! sounds (portal field 0x21 + 0/1/2), and NPCs opening doors.

use std::collections::BTreeMap;

use arcanum_formats::art_id::ArtId;
use arcanum_formats::map::TileLoc;
use arcanum_formats::object::{Object, Prototypes, TYPE_PORTAL};
use bevy_ecs::prelude::*;

use crate::{LedgerDelta, LoadedMap, MapObjectId, NetId, Npc, OPF_JAMMED, OPF_LOCKED, OPF_MAGICALLY_HELD, Outbox, PlayerControlled, PlayerId, Position, Session, SessionPlayer, Walking, WorldState};

/// How near (in tiles, a radius: [`crate::within_radius`]) a PC must be to use a door
/// (0x428930: 2 for portals).
pub const USE_RANGE: u32 = 2;
/// Ticks an opened door waits before it closes itself (1500 ms).
pub const AUTO_CLOSE_TICKS: u32 = 45;
/// `portal_flags`: stays open.
pub const OPF_STICKY: i32 = 0x100;
/// `portal_flags`: broken, so never locked.
pub const OPF_BUSTED: i32 = 0x80;
/// `portal_flags`: never locked.
pub const OPF_NEVER_LOCKED: i32 = 0x8;
/// `portal_flags`: locked only by day (7:00 to 21:00).
pub const OPF_LOCKED_DAY: i32 = 0x20;
/// `portal_flags`: locked only by night.
pub const OPF_LOCKED_NIGHT: i32 = 0x40;

/// Whether `hour` counts as day for day and night locks (0x441d97): 7:00 to 21:59.
pub fn lock_day(hour: u32) -> bool {
    (7..=21).contains(&hour)
}

/// Whether a door with `flags` is locked (0x441d40), by day ([`lock_day`]) or not. Busted and
/// never-locked doors are not; a day or night lock counts only in its hours; otherwise the
/// `LOCKED` bit decides.
pub fn locked(flags: i32, day: bool) -> bool {
    if flags & (OPF_BUSTED | OPF_NEVER_LOCKED) != 0 {
        return false;
    }
    if flags & OPF_LOCKED_DAY != 0 {
        return day;
    }
    if flags & OPF_LOCKED_NIGHT != 0 {
        return !day;
    }
    flags & OPF_LOCKED != 0
}

/// The frames a door shows opening, in order (0x4f0950); closing plays them back to 0.
pub fn open_frames(art: ArtId) -> Vec<u32> {
    if art.0 & 0x400 == 0 {
        return vec![1];
    }
    let rotation = (art.0 >> 11) & 6;
    if matches!(rotation, 0 | 6) { vec![4, 5, 6] } else { vec![1, 2, 3] }
}

/// The frame a door's art shows.
pub fn portal_frame(art: ArtId) -> u32 {
    (art.0 >> 14) & 0x1f
}

/// `art` showing `frame`.
pub fn with_frame(art: ArtId, frame: u32) -> ArtId {
    ArtId((art.0 & !(0x1f << 14)) | (frame & 0x1f) << 14)
}

/// The art of a door opened or closed.
pub fn portal_art(art: ArtId, open: bool) -> ArtId {
    with_frame(art, if open { *open_frames(art).last().expect("never empty") } else { 0 })
}

/// Whether a door is open: any frame but 0, or art bit 0x200 (as the blocking code reads it).
pub fn portal_open(art: ArtId) -> bool {
    art.0 & 0x200 != 0 || portal_frame(art) != 0
}

/// Why a door cannot be opened or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorRefusal {
    NotADoor,
    Jammed,
    MagicallyHeld,
    Locked,
}

/// Whether `door` can be opened (or, when open, closed) by a critter carrying `keys`
/// (0x4aeb70).
pub fn door_check(door: &Object, prototypes: &Prototypes, day: bool, keys: Option<&crate::Inventory>) -> Result<(), DoorRefusal> {
    if door.object_type != TYPE_PORTAL {
        return Err(DoorRefusal::NotADoor);
    }
    let Some(art) = prototypes.art_id(door) else { return Err(DoorRefusal::NotADoor) };
    let flags = prototypes.int(door, "portal_flags").unwrap_or(0);
    if flags & OPF_MAGICALLY_HELD != 0 {
        return Err(DoorRefusal::MagicallyHeld);
    }
    if portal_open(art) {
        return Ok(());
    }
    if flags & OPF_JAMMED != 0 {
        return Err(DoorRefusal::Jammed);
    }
    let lock = prototypes.int(door, "portal_key_id").unwrap_or(0);
    if locked(flags, day) && !keys.is_some_and(|k| k.has_key(lock)) { Err(DoorRefusal::Locked) } else { Ok(()) }
}

impl LoadedMap {
    /// A sector object, if its sector is loaded.
    pub fn map_object(&self, id: MapObjectId) -> Option<&Object> {
        self.sectors.get(&id.sector)?.objects.get(id.index as usize)
    }

    /// Opens or closes door `id`: its art frame, the map's blocking and [`LoadedMap::changes`].
    /// Returns false if it is not a loaded door.
    pub fn set_portal(&mut self, id: MapObjectId, open: bool) -> bool {
        self.changes.entry(id.sector).or_default().portals.insert(id.index, open);
        self.apply_portal(id, open)
    }

    /// Shows door `id` open or closed and updates the blocking of its sector.
    fn apply_portal(&mut self, id: MapObjectId, open: bool) -> bool {
        let prototypes = self.prototypes.clone();
        let Some(sector) = self.sectors.get_mut(&id.sector) else { return false };
        let Some(door) = sector.objects.get_mut(id.index as usize).filter(|o| o.object_type == TYPE_PORTAL) else { return false };
        let Some(art) = prototypes.art_id(door) else { return false };
        door.set_int("current_aid", portal_art(art, open).0 as i32);
        self.rebuild_blocking(id.sector);
        true
    }

    /// An object's `flags`, its own or its prototype's.
    pub fn object_flags(&self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool) -> Option<i32> {
        let object = if mobile { self.mobiles.get(&sector)?.get(index as usize)? } else { self.sectors.get(&sector)?.objects.get(index as usize)? };
        Some(self.prototypes.int(object, "flags").unwrap_or(0))
    }

    /// Sets an object's `flags` (a script's toggle, 0x4efee0 / 0x4eff50): kept in
    /// [`LoadedMap::changes`], and the sector's blocking follows. False if it is not loaded.
    pub fn set_object_flags(&mut self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool, flags: i32) -> bool {
        let delta = self.changes.entry(sector).or_default();
        if mobile { delta.mobile_flags.insert(index, flags) } else { delta.object_flags.insert(index, flags) };
        self.apply_object_flags(sector, index, mobile, flags)
    }

    fn apply_object_flags(&mut self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool, flags: i32) -> bool {
        let object = if mobile { self.mobiles.get_mut(&sector).and_then(|m| m.get_mut(index as usize)) } else { self.sectors.get_mut(&sector).and_then(|s| s.objects.get_mut(index as usize)) };
        let Some(object) = object else { return false };
        object.set_int("flags", flags);
        if self.sectors.contains_key(&sector) {
            self.rebuild_blocking(sector);
        }
        true
    }

    fn object_mut(&mut self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool) -> Option<&mut arcanum_formats::object::Object> {
        if mobile { self.mobiles.get_mut(&sector)?.get_mut(index as usize) } else { self.sectors.get_mut(&sector)?.objects.get_mut(index as usize) }
    }

    /// An object (its sector's or a mobile one), if it is loaded.
    pub fn object_at(&self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool) -> Option<&arcanum_formats::object::Object> {
        if mobile { self.mobiles.get(&sector)?.get(index as usize) } else { self.sectors.get(&sector)?.objects.get(index as usize) }
    }

    /// Sets an object's art (`current_aid`, action 0x3b), kept in [`LoadedMap::changes`].
    pub fn set_object_art(&mut self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool, art: u32) {
        let delta = self.changes.entry(sector).or_default();
        if mobile { delta.mobile_art.insert(index, art) } else { delta.object_art.insert(index, art) };
        if let Some(o) = self.object_mut(sector, index, mobile) {
            o.set_int("current_aid", art as i32);
        }
    }

    /// Makes an object a new instance of prototype `prototype` in its place (action 0x28), kept
    /// in [`LoadedMap::changes`]. False if there is no such prototype or object.
    pub fn transform_object(&mut self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool, prototype: u32) -> bool {
        let delta = self.changes.entry(sector).or_default();
        // The old object's changes go with it.
        if mobile {
            delta.mobile_protos.insert(index, prototype);
            delta.mobile_flags.remove(&index);
            delta.mobile_art.remove(&index);
            delta.npc_flags.remove(&index);
        } else {
            delta.object_protos.insert(index, prototype);
            delta.object_flags.remove(&index);
            delta.object_art.remove(&index);
        }
        self.apply_transform(sector, index, mobile, prototype)
    }

    fn apply_transform(&mut self, sector: arcanum_formats::map::SectorId, index: u32, mobile: bool, prototype: u32) -> bool {
        let Some(proto) = self.prototypes.get(prototype).cloned() else { return false };
        let Some(old) = self.object_mut(sector, index, mobile) else { return false };
        let location = match old.field("location") {
            Some(arcanum_formats::object::Value::Int64(Some(l))) => *l,
            _ => return false,
        };
        let Some(new) = arcanum_formats::object::Object::instance_of(&proto, old.id, location) else { return false };
        *old = new;
        if self.sectors.contains_key(&sector) {
            self.rebuild_blocking(sector);
        }
        true
    }

    /// Whether NPC `index` of `sector` was killed.
    pub fn is_dead(&self, sector: arcanum_formats::map::SectorId, index: u32) -> bool {
        self.changes.get(&sector).is_some_and(|c| c.dead.contains(&index))
    }

    /// An NPC's `npc_flags` (its own or its prototype's), if it is loaded.
    pub fn npc_flags(&self, sector: arcanum_formats::map::SectorId, index: u32) -> Option<i32> {
        let object = self.mobiles.get(&sector)?.get(index as usize)?;
        Some(self.prototypes.int(object, "npc_flags").unwrap_or(0))
    }

    /// Moves an NPC's day or night standpoint (`npc_standpoint_day` / `_night`, actions 0x2d /
    /// 0x2e), kept in [`LoadedMap::changes`]. False when it is not an NPC of this map or did not
    /// move.
    pub fn set_standpoint(&mut self, sector: arcanum_formats::map::SectorId, index: u32, to: TileLoc, night: bool) -> bool {
        let Some(object) = self.mobiles.get_mut(&sector).and_then(|m| m.get_mut(index as usize)) else { return false };
        if object.object_type != arcanum_formats::object::TYPE_NPC {
            return false;
        }
        set_standpoint(object, to, night);
        let changes = self.changes.entry(sector).or_default();
        let kept = if night { &mut changes.night_standpoints } else { &mut changes.standpoints };
        kept.insert(index, to) != Some(to)
    }

    /// Sets an NPC's `npc_flags` ([`crate::follow`]), kept in [`LoadedMap::changes`].
    pub fn set_npc_flags(&mut self, sector: arcanum_formats::map::SectorId, index: u32, flags: i32) {
        self.changes.entry(sector).or_default().npc_flags.insert(index, flags);
        if let Some(object) = self.mobiles.get_mut(&sector).and_then(|m| m.get_mut(index as usize)) {
            object.set_int("npc_flags", flags);
        }
    }

    /// Applies the saved changes of sector `id` (just loaded) to its objects.
    pub fn apply_changes(&mut self, id: arcanum_formats::map::SectorId) {
        let Some(delta) = self.changes.get(&id).cloned() else { return };
        // A new object first: the other changes apply to it.
        for (index, prototype) in delta.object_protos.clone() {
            self.apply_transform(id, index, false, prototype);
        }
        for (index, prototype) in delta.mobile_protos.clone() {
            self.apply_transform(id, index, true, prototype);
        }
        for (index, flags) in delta.object_flags {
            self.apply_object_flags(id, index, false, flags);
        }
        for (index, flags) in delta.mobile_flags {
            self.apply_object_flags(id, index, true, flags);
        }
        for (index, art) in delta.object_art {
            if let Some(o) = self.object_mut(id, index, false) {
                o.set_int("current_aid", art as i32);
            }
        }
        for (index, art) in delta.mobile_art {
            if let Some(o) = self.object_mut(id, index, true) {
                o.set_int("current_aid", art as i32);
            }
        }
        for (index, flags) in delta.npc_flags {
            if let Some(object) = self.mobiles.get_mut(&id).and_then(|m| m.get_mut(index as usize)) {
                object.set_int("npc_flags", flags);
            }
        }
        for (night, kept) in [(false, delta.standpoints), (true, delta.night_standpoints)] {
            for (index, at) in kept {
                if let Some(object) = self.mobiles.get_mut(&id).and_then(|m| m.get_mut(index as usize)) {
                    set_standpoint(object, at, night);
                }
            }
        }
        for (index, open) in delta.portals {
            self.apply_portal(MapObjectId { sector: id, index }, open);
        }
        for (index, flags) in delta.portal_flags {
            self.apply_lock_flags(crate::LockTarget::Door(MapObjectId { sector: id, index }), flags);
        }
        for (index, flags) in delta.container_flags {
            self.apply_lock_flags(crate::LockTarget::Container(crate::GroundItemId::Container { sector: id, index }), flags);
        }
    }
}

/// Sets `npc_standpoint_day` or `_night` (a packed location: x low, y high).
fn set_standpoint(object: &mut arcanum_formats::object::Object, at: TileLoc, night: bool) {
    let field = if night { "npc_standpoint_night" } else { "npc_standpoint_day" };
    object.set_value(field, arcanum_formats::object::Value::Int64(Some(i64::from(at.x) | (i64::from(at.y) << 32))));
}

/// Where a door is: its object's tile.
fn door_tile(map: &LoadedMap, id: MapObjectId) -> Option<TileLoc> {
    crate::object_location(map.map_object(id)?)
}

/// A player's request to use a door or scenery, waiting for their PC to be in range. `seq` is the move it
/// came with: a newer move drops it.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingUse {
    pub door: MapObjectId,
    pub seq: u32,
}

/// Doors opened that will close themselves, with the ticks left.
#[derive(Resource, Debug, Default)]
pub struct ClosingDoors(pub BTreeMap<MapObjectId, u32>);

/// Use requests taken from the inbox by `handle_intents`.
#[derive(Resource, Debug, Default)]
pub struct UseRequests(pub Vec<(PlayerId, MapObjectId, u32)>);

fn within(a: TileLoc, b: TileLoc, range: u32) -> bool {
    crate::within_radius(a, b, range)
}

/// Opens or closes `door` for everyone, if it can be, by a critter carrying `keys`. Returns
/// whether it changed.
#[allow(clippy::too_many_arguments)]
fn use_door(map: &mut LoadedMap, world: &mut WorldState, closing: &mut ClosingDoors, session: &Session, outbox: &mut Outbox, door: MapObjectId, keys: Option<&crate::Inventory>) -> bool {
    let Some(object) = map.map_object(door) else { return false };
    if door_check(object, &map.prototypes, map.lock_day, keys).is_err() {
        return false;
    }
    let Some(art) = map.prototypes.art_id(object) else { return false };
    let open = !portal_open(art);
    let sticky = map.prototypes.int(object, "portal_flags").unwrap_or(0) & OPF_STICKY != 0;
    map.set_portal(door, open);
    if let Some(save) = world.0.as_mut() {
        save.sector_changes.entry(door.sector).or_default().portals.insert(door.index, open);
    }
    if open && !sticky {
        closing.0.insert(door, AUTO_CLOSE_TICKS);
    } else {
        closing.0.remove(&door);
    }
    let delta = LedgerDelta::PortalChanged { door, open };
    for (&player, state) in &session.players {
        if matches!(state, SessionPlayer::InWorld(_)) {
            outbox.0.push((player, delta.clone()));
        }
    }
    true
}

type UserQuery<'a> = (Entity, &'a NetId, &'a Position, Option<&'a Walking>, Option<&'a PendingUse>, Option<&'a crate::Inventory>, &'a PlayerControlled);
/// PCs and NPCs.
type Critter = Or<(With<PlayerControlled>, With<Npc>)>;
/// A PC with a use waiting: its tile and whether it walks.
type Waiting = (Entity, TileLoc, bool, PendingUse);

/// Takes use requests: each becomes the PC's [`PendingUse`]. Then uses the doors whose PC is in
/// range, and drops requests whose PC stopped out of range.
#[allow(clippy::too_many_arguments)]
pub fn use_portals(
    mut commands: Commands,
    mut requests: ResMut<UseRequests>,
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    mut closing: ResMut<ClosingDoors>,
    session: Res<Session>,
    mut outbox: ResMut<Outbox>,
    pcs: Query<UserQuery, With<PlayerControlled>>,
    mut scripts: crate::ScriptHost,
    mut beds: ResMut<crate::BedRequests>,
) {
    let mut pending: Vec<Waiting> = pcs.iter().filter_map(|(e, _, p, w, u, ..)| u.map(|u| (e, p.0, w.is_some(), *u))).collect();
    for (player, door, seq) in requests.0.drain(..) {
        let Some(SessionPlayer::InWorld(net_id)) = session.players.get(&player) else { continue };
        let Some((entity, _, &Position(at), walking, ..)) = pcs.iter().find(|pc| pc.1 == net_id) else { continue };
        pending.retain(|p| p.0 != entity);
        pending.push((entity, at, walking.is_some(), PendingUse { door, seq }));
    }
    for (entity, at, walking, use_) in pending {
        let in_range = door_tile(&map, use_.door).is_some_and(|t| within(at, t, USE_RANGE));
        if in_range {
            commands.entity(entity).remove::<PendingUse>();
            // The door's "use" script first (0x428c17): it may handle the use itself.
            let Ok((.., keys, player)) = pcs.get(entity) else { continue };
            // Scenery: a bed opens the sleep window on it, which runs its script (0x428c74);
            // other scenery has only its script.
            if map.map_object(use_.door).is_some_and(|o| o.object_type == arcanum_formats::object::TYPE_SCENERY) {
                if map.map_object(use_.door).is_some_and(|o| crate::is_bed(o, &map.prototypes)) {
                    beds.0.push((player.0, use_.door));
                } else {
                    scripts.hook(&mut world, &mut map, &mut outbox, crate::ScriptObject::Map(use_.door), crate::EVENT_USE, player.0);
                }
                continue;
            }
            if scripts.hook(&mut world, &mut map, &mut outbox, crate::ScriptObject::Map(use_.door), crate::EVENT_USE, player.0) == crate::HookResult::SkipDefault {
                continue;
            }
            // Opening a shut door: its "unlock" script before the lock check (0x4aeb70) may deny it.
            let shut = map.map_object(use_.door).and_then(|o| map.prototypes.art_id(o)).is_some_and(|art| !portal_open(art));
            if shut && scripts.hook(&mut world, &mut map, &mut outbox, crate::ScriptObject::Map(use_.door), crate::EVENT_UNLOCK, player.0) == crate::HookResult::SkipDefault {
                continue;
            }
            use_door(&mut map, &mut world, &mut closing, &session, &mut outbox, use_.door, keys);
        } else if walking {
            commands.entity(entity).insert(use_);
        } else {
            commands.entity(entity).remove::<PendingUse>();
        }
    }
}

/// Counts down doors that close themselves; one with a critter within [`USE_RANGE`] waits
/// another round.
pub fn close_doors(
    mut map: ResMut<LoadedMap>,
    mut world: ResMut<WorldState>,
    mut closing: ResMut<ClosingDoors>,
    session: Res<Session>,
    mut outbox: ResMut<Outbox>,
    critters: Query<&Position, Critter>,
) {
    let mut due = Vec::new();
    for (&door, ticks) in closing.0.iter_mut() {
        *ticks = ticks.saturating_sub(1);
        if *ticks == 0 {
            due.push(door);
        }
    }
    for door in due {
        let Some(tile) = door_tile(&map, door) else {
            closing.0.remove(&door);
            continue;
        };
        if critters.iter().any(|p| within(p.0, tile, USE_RANGE)) {
            closing.0.insert(door, AUTO_CLOSE_TICKS);
            continue;
        }
        let open = map.map_object(door).and_then(|o| map.prototypes.art_id(o)).is_some_and(portal_open);
        closing.0.remove(&door);
        if open {
            use_door(&mut map, &mut world, &mut closing, &session, &mut outbox, door, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_follow_the_flags_and_the_hour() {
        assert!(locked(OPF_LOCKED, true));
        assert!(!locked(OPF_LOCKED | OPF_NEVER_LOCKED, true), "the prototypes' default 9");
        assert!(!locked(OPF_LOCKED | OPF_BUSTED, true));
        assert!(!locked(OPF_LOCKED | OPF_LOCKED_NIGHT, true), "by day");
        assert!(locked(OPF_LOCKED | OPF_LOCKED_NIGHT, false), "by night");
        assert!(locked(OPF_LOCKED_DAY, true));
        assert!(!locked(OPF_LOCKED_DAY, false));
        assert!(!lock_day(6) && lock_day(7) && lock_day(21) && !lock_day(22));
    }

    #[test]
    fn doors_open_to_their_last_frame_and_close_to_zero() {
        let plain = ArtId(0x3000_0000 | 3 << 11);
        assert_eq!(open_frames(plain), vec![1]);
        assert_eq!(portal_frame(portal_art(plain, true)), 1);
        let swing = ArtId(0x3000_0000 | 0x400 | 3 << 11);
        assert_eq!(open_frames(swing), vec![1, 2, 3]);
        let swing_up = ArtId(0x3000_0000 | 0x400 | 7 << 11);
        assert_eq!(open_frames(swing_up), vec![4, 5, 6], "rotation 7 counts as 6");
        let open = portal_art(swing_up, true);
        assert_eq!(portal_frame(open), 6);
        assert!(portal_open(open));
        assert!(!portal_open(portal_art(open, false)));
        assert_eq!(portal_art(open, false).0 & !(0x1f << 14), swing_up.0, "only the frame changes");
    }

    mod host {
        use std::path::Path;
        use std::sync::{Arc, RwLock};

        use arcanum_formats::map::SectorId;
        use arcanum_formats::object::Value;
        use arcanum_formats::test_util::{ObjectBuilder, SectorBuilder};
        use arcanum_rules::{CharacterBlueprint, CharacterSheet, RulesData};
        use arcanum_vfs::Vfs;

        use super::super::*;
        use crate::{PlayerIntent, PlayerSave, SimApp, SimVfs, WorldSave, step_allowed};

        const MAP: &str = "Doors";

        fn t(x: u32, y: u32) -> TileLoc {
            TileLoc { x, y }
        }

        /// A sector (1, 1) with a plain door at (72, 80) facing direction 3 (towards (72, 81)),
        /// a locked one at (90, 80) with key 20, and a night-locked one at (110, 80).
        fn write_map(dir: &Path) {
            let folder = dir.join("maps").join(MAP);
            std::fs::create_dir_all(&folder).unwrap();
            let mut prp = Vec::new();
            for v in [7u32, 1] {
                prp.extend(v.to_le_bytes());
            }
            for v in [256u64, 256] {
                prp.extend(v.to_le_bytes());
            }
            std::fs::write(folder.join("map.prp"), prp).unwrap();
            let at = |x: u32, y: u32| Value::Int64(Some(i64::from(x) | (i64::from(y) << 32)));
            let door = |x: u32, flags: i32| {
                let x: u32 = x;
                ObjectBuilder::new(TYPE_PORTAL)
                    .field("current_aid", Value::Int32((0x3000_0000u32 | 3 << 11) as i32))
                    .field("location", at(x, 80))
                    .field("portal_flags", Value::Int32(flags))
                    .field("portal_key_id", Value::Int32(if flags & OPF_LOCKED != 0 { 20 } else { 0 }))
                    .build()
            };
            let mut sector = SectorBuilder::new();
            // A bed (scenery, art category 22) at (120, 120).
            let bed = ObjectBuilder::new(arcanum_formats::object::TYPE_SCENERY)
                .field("current_aid", Value::Int32((0x4000_0000u32 | crate::BED_CATEGORY << 6) as i32))
                .field("location", Value::Int64(Some(120 | (120i64 << 32))))
                .build();
            sector.objects = vec![door(72, 0), door(90, OPF_LOCKED), door(110, OPF_LOCKED | OPF_LOCKED_NIGHT), bed];
            std::fs::write(folder.join(SectorId::new(1, 1).file_name()), sector.build()).unwrap();
        }

        fn booted(dir: &Path, players: &[(u64, TileLoc)], changes: BTreeMap<SectorId, crate::SectorDelta>) -> SimApp {
            let mut save = WorldSave::new_game(&crate::StartMap { map: MAP.into(), position: t(0, 0), clock: Default::default() });
            save.sector_changes = changes;
            for &(id, position) in players {
                let sheet = CharacterSheet { name: "A".into(), ..Default::default() };
                save.players.insert(PlayerId(id), PlayerSave { blueprint: CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None }, position, inventory: None, script_state: Default::default(), worn: None });
            }
            let mut vfs = Vfs::new();
            vfs.mount_dir(dir);
            let mut sim = SimApp::new(RulesData::default());
            sim.use_manual_clock();
            sim.app.insert_resource(SimVfs(Arc::new(RwLock::new(vfs))));
            sim.boot(save);
            sim.update();
            sim.update();
            sim
        }

        const DOOR: MapObjectId = MapObjectId { sector: SectorId(1 << 26 | 1), index: 0 };
        const LOCKED: MapObjectId = MapObjectId { sector: SectorId(1 << 26 | 1), index: 1 };
        const NIGHT_LOCKED: MapObjectId = MapObjectId { sector: SectorId(1 << 26 | 1), index: 2 };
        const BED: MapObjectId = MapObjectId { sector: SectorId(1 << 26 | 1), index: 3 };

        #[test]
        fn a_door_runs_its_use_script_first_and_the_script_can_keep_it_shut() {
            use crate::script::tests::{always, block, line, n, op};
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            // The door carries script 500 for "use" (as a save that already changed it).
            let mut changes = BTreeMap::new();
            let delta: &mut crate::SectorDelta = changes.entry(DOOR.sector).or_default();
            delta.object_scripts.insert(DOOR.index, BTreeMap::from([(crate::EVENT_USE, crate::Attachment::new(0, 0, 500))]));
            let mut sim = booted(dir.path(), &[(7, t(71, 80))], changes);
            sim.take_deltas();
            // Count the uses, set global flag 9 and PC flag 4; the first use is the script's own.
            let script = arcanum_formats::script::Script {
                description: String::new(),
                flags: 0,
                lines: vec![
                    always(block(0xb, &[op(0, 0), op(0, 0), n(1)])),
                    always(block(0x64, &[n(9)])),
                    always(block(0xa, &[op(6, 4), n(1)])),
                    line(block(4, &[op(0, 0), n(1)]), block(1, &[]), block(2, &[])),
                ],
            };
            sim.app.world_mut().resource_mut::<crate::Scripts>().0.get_or_insert_with(Default::default).insert(500, script);
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 0), 7), None, "the script handled it: shut");
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 1), 7), Some(true), "then the default: open");
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            assert_eq!(save.sector_changes[&DOOR.sector].object_scripts[&DOOR.index][&crate::EVENT_USE].counters[0], 2, "counted and saved");
            assert!(save.scripts.flags.contains(&9));
            assert!(save.players[&PlayerId(7)].script_state.flags.contains(&4), "the triggering PC's flag");
            assert!(sim.app.world().resource::<crate::ScriptReports>().0.is_empty());
        }

        #[test]
        fn a_doors_unlock_script_can_keep_it_shut() {
            use crate::script::tests::{block, line, n};
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut changes = BTreeMap::new();
            let delta: &mut crate::SectorDelta = changes.entry(DOOR.sector).or_default();
            delta.object_scripts.insert(DOOR.index, BTreeMap::from([(crate::EVENT_UNLOCK, crate::Attachment::new(0, 0, 740))]));
            let mut sim = booted(dir.path(), &[(7, t(71, 80))], changes);
            // Shut until global flag 5 is set.
            let lines = vec![line(block(0x2b, &[n(5)]), block(2, &[]), block(1, &[]))];
            sim.app.world_mut().resource_mut::<crate::Scripts>().0.get_or_insert_with(Default::default).insert(740, arcanum_formats::script::Script { description: String::new(), flags: 0, lines });
            sim.take_deltas();
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 0), 7), None, "denied");
            sim.app.world_mut().resource_mut::<WorldState>().0.as_mut().unwrap().scripts.set_flag(5, true);
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 1), 7), Some(true), "allowed");
        }

        #[test]
        fn a_trapped_door_springs_once_and_a_spotted_trap_can_be_disarmed() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            // A mechanical trap of 4 to 4 damage on the door's use.
            let trapped = crate::Attachment { flags: 0, counters: [4, 4, 0, 0], number: 30001 };
            let mut changes = BTreeMap::new();
            let delta: &mut crate::SectorDelta = changes.entry(DOOR.sector).or_default();
            delta.object_scripts.insert(DOOR.index, BTreeMap::from([(crate::EVENT_USE, trapped)]));
            let mut sim = booted(dir.path(), &[(7, t(71, 80))], changes);
            let trap = arcanum_formats::script::Script { description: "Mechanical trap".into(), flags: crate::TRAP_MECHANICAL | arcanum_formats::script::RUN_ONCE, lines: vec![] };
            sim.app.world_mut().resource_mut::<crate::Scripts>().0.get_or_insert_with(Default::default).insert(30001, trap);
            sim.take_deltas();
            // No Spot Trap: it springs, the door stays shut, and the trap is gone.
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 0), 7), None, "sprung: the use does not happen");
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            assert_eq!(save.players[&PlayerId(7)].blueprint.sheet.hp_damage, 4);
            assert_eq!(save.sector_changes[&DOOR.sector].object_scripts[&DOOR.index][&crate::EVENT_USE].number, 0, "run once");
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 1), 7), Some(true), "then it opens");
            // Trapped again, and spotted: a skilled PC disarms it.
            {
                let mut map = sim.app.world_mut().resource_mut::<LoadedMap>();
                map.set_attachment(crate::ScriptObject::Map(DOOR), crate::EVENT_USE, trapped);
                let flags = map.object_flags(DOOR.sector, DOOR.index, false).unwrap();
                map.set_object_flags(DOOR.sector, DOOR.index, false, flags | crate::OF_TRAP_SPOTTED);
            }
            {
                let world = sim.app.world_mut();
                let mut sheet = world.query::<&mut crate::Sheet>().single_mut(world).unwrap();
                sheet.0.base = [20; 8];
                sheet.0.skills[crate::SKILL_DISARM_TRAPS] = 5;
            }
            let mut tries = 0;
            loop {
                tries += 1;
                sim.receive([(PlayerId(7), PlayerIntent::DisarmTrap { target: crate::LockTarget::Door(DOOR), seq: 2 })]);
                sim.update();
                let deltas = sim.take_deltas();
                if deltas.contains(&(PlayerId(7), LedgerDelta::SkillResult { message: crate::DISARM_SUCCEEDED })) {
                    break;
                }
                assert!(tries < 5, "95 %: {deltas:?}");
            }
            assert_eq!(sim.app.world().resource::<LoadedMap>().attachment(crate::ScriptObject::Map(DOOR), crate::EVENT_USE).number, 0, "disarmed");
            // Nothing there now: a disarm fails.
            sim.receive([(PlayerId(7), PlayerIntent::DisarmTrap { target: crate::LockTarget::Door(DOOR), seq: 3 })]);
            sim.update();
            assert!(sim.take_deltas().contains(&(PlayerId(7), LedgerDelta::SkillResult { message: crate::DISARM_FAILED })));
        }

        #[test]
        fn a_bed_refuses_without_its_script_saying_so_then_sleeps_twice_as_well() {
            use crate::script::tests::{always, block};
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(119, 120))], BTreeMap::new());
            sim.take_deltas();
            assert!(sim.app.world().resource::<LoadedMap>().map_object(BED).is_some_and(|o| crate::is_bed(o, &sim.app.world().resource::<LoadedMap>().prototypes)));
            let use_bed = |sim: &mut SimApp, seq: u32| {
                sim.receive([(PlayerId(7), PlayerIntent::UsePortal { door: BED, seq })]);
                sim.update();
                sim.take_deltas()
            };
            // No script: the default, "You cannot sleep here. Find an inn and pay for a bed."
            let deltas = use_bed(&mut sim, 0);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::RestRefused { message: crate::CANNOT_SLEEP_HERE })), "{deltas:?}");
            // A script that handles the use (a paid inn bed): the sleep window opens on the bed.
            let script = arcanum_formats::script::Script { description: String::new(), flags: 0, lines: vec![always(block(1, &[]))] };
            sim.app.world_mut().resource_mut::<crate::Scripts>().0.get_or_insert_with(Default::default).insert(600, script);
            sim.app.world_mut().resource_mut::<LoadedMap>().set_attachment(crate::ScriptObject::Map(BED), crate::EVENT_USE, crate::Attachment::new(0, 0, 600));
            let deltas = use_bed(&mut sim, 1);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::RestWindow { mode: crate::RestMode::Sleep })), "{deltas:?}");
            sim.receive([(PlayerId(7), PlayerIntent::Rest { option: crate::RestOption::Hours(1) })]);
            sim.update();
            let resting = {
                let world = sim.app.world_mut();
                world.query_filtered::<&crate::Resting, With<PlayerControlled>>().single(world).ok().copied()
            };
            assert_eq!(resting.and_then(|r| r.bed), Some(BED), "resting in the bed: twice the healing");
            let mut ended = false;
            for _ in 0..crate::FIRST_STEP_TICKS + 1 {
                sim.update();
                ended |= sim.take_deltas().contains(&(PlayerId(7), LedgerDelta::RestEnded));
            }
            assert!(ended);
        }

        #[test]
        fn a_night_lock_holds_only_at_night() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            // 21:59 game time: a day; a few real seconds later it is 22:00, night.
            let mut sim = booted(dir.path(), &[(7, t(109, 80))], BTreeMap::new());
            sim.app.world_mut().resource_mut::<crate::GameClock>().now = crate::GameTime { days: 0, ms: 22 * crate::HOUR_MS - 4000 };
            sim.update();
            sim.take_deltas();
            assert!(step_allowed(sim.app.world().resource::<LoadedMap>(), t(110, 80), t(110, 81)), "by day: passable");
            for _ in 0..30 {
                sim.update();
            }
            assert!(!step_allowed(sim.app.world().resource::<LoadedMap>(), t(110, 80), t(110, 81)), "by night: locked");
            assert_eq!(changed(&use_door(&mut sim, 7, NIGHT_LOCKED, 0), 7), None, "and it does not open");
        }

        fn use_door(sim: &mut SimApp, player: u64, door: MapObjectId, seq: u32) -> Vec<(PlayerId, LedgerDelta)> {
            sim.receive([(PlayerId(player), PlayerIntent::UsePortal { door, seq })]);
            sim.update();
            sim.take_deltas()
        }

        fn changed(deltas: &[(PlayerId, LedgerDelta)], to: u64) -> Option<bool> {
            deltas.iter().find_map(|(p, d)| match d {
                LedgerDelta::PortalChanged { open, .. } if p.0 == to => Some(*open),
                _ => None,
            })
        }

        fn passes(sim: &SimApp) -> bool {
            step_allowed(sim.app.world().resource::<LoadedMap>(), t(72, 80), t(72, 81))
        }

        #[test]
        fn a_pc_in_range_opens_a_door_for_everyone_and_it_closes_itself() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            assert_eq!(DOOR.sector, SectorId::new(1, 1));
            let mut sim = booted(dir.path(), &[(7, t(70, 80)), (9, t(100, 100))], BTreeMap::new());
            sim.take_deltas();
            assert!(passes(&sim), "a closed, unlocked door does not block the PC");
            let deltas = use_door(&mut sim, 7, DOOR, 0);
            assert_eq!((changed(&deltas, 7), changed(&deltas, 9)), (Some(true), Some(true)));
            let map = sim.app.world().resource::<LoadedMap>();
            assert!(portal_open(map.prototypes.art_id(map.map_object(DOOR).unwrap()).unwrap()));
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            assert!(save.sector_changes[&DOOR.sector].portals[&0], "saved");
            // The PC stands within range: the door stays open.
            for _ in 0..AUTO_CLOSE_TICKS * 2 {
                sim.update();
            }
            assert_eq!(changed(&sim.take_deltas(), 7), None);
            // It walks off; the door closes once its wait is up.
            sim.receive([(PlayerId(7), PlayerIntent::Move { seq: 1, path: (64..=70).rev().map(|x| t(x, 80)).collect() })]);
            let mut closed = None;
            for _ in 0..AUTO_CLOSE_TICKS * 6 {
                sim.update();
                closed = closed.or(changed(&sim.take_deltas(), 9));
            }
            assert_eq!(closed, Some(false));
        }

        #[test]
        fn locked_doors_stay_shut_and_block() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80))], BTreeMap::new());
            sim.take_deltas();
            assert_eq!(changed(&use_door(&mut sim, 7, LOCKED, 0), 7), None);
            let map = sim.app.world().resource::<LoadedMap>();
            assert!(!step_allowed(map, t(90, 80), t(90, 81)), "locked: the edge blocks");
        }

        #[test]
        fn picking_a_lock_unlocks_the_door_for_everyone_and_is_saved() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80)), (9, t(100, 100))], BTreeMap::new());
            sim.take_deltas();
            let pick = |sim: &mut SimApp| {
                sim.receive([(PlayerId(7), PlayerIntent::PickLock { tool: None, target: crate::LockTarget::Door(LOCKED), seq: 0 })]);
                sim.update();
                sim.take_deltas()
            };
            // No skill: a 0 % chance, which fails.
            let deltas = pick(&mut sim);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::SkillResult { message: crate::PICK_FAILED })), "{deltas:?}");
            assert!(!deltas.iter().any(|(p, _)| p.0 == 9), "the other player hears nothing of a plain failure");
            // A master of the skill (level 20, 95 %): the lock gives.
            {
                let world = sim.app.world_mut();
                let mut q = world.query::<(&PlayerControlled, &mut crate::Sheet)>();
                let (_, mut sheet) = q.iter_mut(world).find(|(p, _)| p.0 == PlayerId(7)).unwrap();
                sheet.0.base[1] = 20;
                sheet.0.skills[crate::SKILL_PICK_LOCKS] = 5;
            }
            let deltas = pick(&mut sim);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::SkillResult { message: crate::PICK_SUCCEEDED })), "{deltas:?}");
            assert!(deltas.contains(&(PlayerId(9), LedgerDelta::LockChanged { target: crate::LockTarget::Door(LOCKED), flags: 0 })));
            assert!(step_allowed(sim.app.world().resource::<LoadedMap>(), t(90, 80), t(90, 81)), "unlocked: passable");
            assert_eq!(changed(&use_door(&mut sim, 7, LOCKED, 1), 7), Some(true), "and it opens");
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            assert_eq!(save.sector_changes[&LOCKED.sector].portal_flags[&1], 0, "saved");
            // Loaded again, the door is still unlocked.
            let sim = booted(dir.path(), &[(7, t(89, 80))], save.sector_changes.clone());
            let map = sim.app.world().resource::<LoadedMap>();
            assert_eq!(map.lock_flags(crate::LockTarget::Door(LOCKED)), Some(0));
        }

        #[test]
        fn the_unlocking_cantrip_unlocks_the_door_and_costs_fatigue() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80)), (9, t(100, 100))], BTreeMap::new());
            sim.take_deltas();
            sim.app.world_mut().resource_mut::<crate::SimRules>().0.spells.spells =
                (0..arcanum_rules::SPELLS).map(|i| arcanum_rules::Spell { name: String::new(), description: String::new(), college: i / 5, cost: 10 }).collect();
            let cast = |sim: &mut SimApp| {
                sim.receive([(PlayerId(7), PlayerIntent::Cast { spell: crate::SPELL_UNLOCKING_CANTRIP, target: crate::LockTarget::Door(LOCKED) })]);
                sim.update();
                sim.take_deltas()
            };
            let set = |sim: &mut SimApp, f: &dyn Fn(&mut arcanum_rules::CharacterSheet)| {
                let world = sim.app.world_mut();
                let mut q = world.query::<(&PlayerControlled, &mut crate::Sheet)>();
                let (_, mut sheet) = q.iter_mut(world).find(|(p, _)| p.0 == PlayerId(7)).unwrap();
                f(&mut sheet.0);
            };
            assert!(cast(&mut sim).iter().all(|(_, d)| !matches!(d, LedgerDelta::LockChanged { .. })), "not known");
            // Known, but too tired: 5 fatigue left, and 20 would take it below -14.
            let max = sim.app.world().resource::<crate::SimRules>().0.derived(&arcanum_rules::CharacterSheet::default()).fatigue;
            sim.app.world_mut().resource_mut::<crate::SimRules>().0.spells.spells[1].cost = 20;
            set(&mut sim, &|s| {
                s.spells[0] = 2;
                s.fatigue_damage = max - 5;
            });
            let deltas = cast(&mut sim);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::SpellResult { message: crate::NOT_ENOUGH_ENERGY })), "{deltas:?}");
            sim.app.world_mut().resource_mut::<crate::SimRules>().0.spells.spells[1].cost = 10;
            set(&mut sim, &|s| s.fatigue_damage = 0);
            let deltas = cast(&mut sim);
            assert!(deltas.contains(&(PlayerId(9), LedgerDelta::LockChanged { target: crate::LockTarget::Door(LOCKED), flags: 0 })), "{deltas:?}");
            let fatigue = deltas.iter().find_map(|(p, d)| match d {
                LedgerDelta::PcStatus { status, .. } if p.0 == 9 => Some(status.fatigue),
                _ => None,
            });
            assert!(fatigue.is_some_and(|f| f.current == f.max - 10), "{fatigue:?}");
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            assert_eq!(save.players[&PlayerId(7)].blueprint.sheet.fatigue_damage, 10, "spent fatigue is saved");
            assert_eq!(save.sector_changes[&LOCKED.sector].portal_flags[&1], 0);
            assert_eq!(changed(&use_door(&mut sim, 7, LOCKED, 1), 7), Some(true), "and it opens");
            // 80 game seconds on, the heal rate comes back.
            let now = sim.app.world().resource::<crate::GameClock>().now;
            sim.app.world_mut().resource_mut::<crate::GameClock>().now = now.plus_ms(crate::RECOVERY_MS + 1000);
            sim.update();
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            let rate = sim.app.world().resource::<crate::SimRules>().0.derived(&save.players[&PlayerId(7)].blueprint.sheet).heal_rate;
            assert_eq!(save.players[&PlayerId(7)].blueprint.sheet.fatigue_damage, (10 - rate).max(0));
        }

        #[test]
        fn a_cast_that_takes_fatigue_below_one_knocks_the_pc_down_until_it_recovers() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80))], BTreeMap::new());
            sim.take_deltas();
            sim.app.world_mut().resource_mut::<crate::SimRules>().0.spells.spells =
                (0..arcanum_rules::SPELLS).map(|i| arcanum_rules::Spell { name: String::new(), description: String::new(), college: i / 5, cost: 10 }).collect();
            {
                let world = sim.app.world_mut();
                let rules = world.resource::<crate::SimRules>().0.clone();
                let mut sheet = world.query_filtered::<&mut crate::Sheet, With<PlayerControlled>>().single_mut(world).unwrap();
                sheet.0.spells[0] = 2;
                sheet.0.fatigue_damage = rules.derived(&sheet.0).fatigue - 5;
            }
            sim.receive([(PlayerId(7), PlayerIntent::Cast { spell: crate::SPELL_UNLOCKING_CANTRIP, target: crate::LockTarget::Door(LOCKED) })]);
            sim.update();
            let down = |sim: &mut SimApp| {
                let world = sim.app.world_mut();
                world.query_filtered::<(), (With<PlayerControlled>, With<crate::Unconscious>)>().iter(world).count() == 1
            };
            assert!(down(&mut sim), "5 - 10: down");
            let walking = |sim: &mut SimApp| {
                let world = sim.app.world_mut();
                world.query_filtered::<(), (With<PlayerControlled>, With<crate::Walking>)>().iter(world).count() == 1
            };
            sim.receive([(PlayerId(7), PlayerIntent::Move { seq: 1, path: vec![t(89, 80), t(88, 80)] })]);
            sim.update();
            assert!(!walking(&mut sim), "an unconscious PC does not walk");
            // Fatigue comes back step by step; above 0 it gets up and walks again.
            for _ in 0..20 {
                let now = sim.app.world().resource::<crate::GameClock>().now;
                sim.app.world_mut().resource_mut::<crate::GameClock>().now = now.plus_ms(crate::RECOVERY_MS);
                sim.update();
            }
            assert!(!down(&mut sim), "awake");
            sim.receive([(PlayerId(7), PlayerIntent::Move { seq: 2, path: vec![t(89, 80), t(88, 80)] })]);
            sim.update();
            assert!(walking(&mut sim));
        }

        #[test]
        fn sleeping_jumps_the_clock_an_hour_a_step_and_heals_fatigue() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80))], BTreeMap::new());
            sim.take_deltas();
            let set_damage = |sim: &mut SimApp, damage: i32| {
                let world = sim.app.world_mut();
                world.query_filtered::<&mut crate::Sheet, With<PlayerControlled>>().single_mut(world).unwrap().0.fatigue_damage = damage;
            };
            let ask = |sim: &mut SimApp, intent: PlayerIntent| {
                sim.receive([(PlayerId(7), intent)]);
                sim.update();
                sim.take_deltas()
            };
            // Unconscious: refused.
            let max = sim.app.world().resource::<crate::SimRules>().0.derived(&arcanum_rules::CharacterSheet::default()).fatigue;
            set_damage(&mut sim, max);
            let deltas = ask(&mut sim, PlayerIntent::RestWindow);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::RestRefused { message: crate::ALREADY_UNCONSCIOUS })), "{deltas:?}");
            // In the wilderness (no town map): sleeping.
            set_damage(&mut sim, 20);
            sim.update();
            let deltas = ask(&mut sim, PlayerIntent::RestWindow);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::RestWindow { mode: crate::RestMode::Sleep })), "{deltas:?}");
            let start = sim.app.world().resource::<crate::GameClock>().now;
            ask(&mut sim, PlayerIntent::Rest { option: crate::RestOption::Hours(2) });
            // Resting, the PC does not walk.
            ask(&mut sim, PlayerIntent::Move { seq: 1, path: vec![t(89, 80), t(88, 80)] });
            let walking = {
                let world = sim.app.world_mut();
                world.query_filtered::<(), (With<PlayerControlled>, With<crate::Walking>)>().iter(world).count()
            };
            assert_eq!(walking, 0, "moves are dropped while resting");
            let mut ended = 0;
            for _ in 0..crate::FIRST_STEP_TICKS + crate::STEP_TICKS {
                sim.update();
                ended += sim.take_deltas().iter().filter(|d| d == &&(PlayerId(7), LedgerDelta::RestEnded)).count();
            }
            assert_eq!(ended, 1);
            let now = sim.app.world().resource::<crate::GameClock>().now;
            let passed = (now.days - start.days) as u64 * crate::DAY_MS as u64 + now.ms as u64 - start.ms as u64;
            assert!((2 * crate::HOUR_MS as u64..2 * crate::HOUR_MS as u64 + 10_000).contains(&passed), "two hours and a few ticks: {passed}");
            let save = sim.app.world().resource::<WorldState>().0.clone().unwrap();
            assert_eq!(save.players[&PlayerId(7)].blueprint.sheet.fatigue_damage, 0, "rested, and the recovery due caught up");
            // Stopped early.
            set_damage(&mut sim, 20);
            ask(&mut sim, PlayerIntent::Rest { option: crate::RestOption::Hours(24) });
            let deltas = ask(&mut sim, PlayerIntent::StopRest);
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::RestEnded)), "{deltas:?}");
            let left = {
                let world = sim.app.world_mut();
                world.query_filtered::<(), With<crate::Resting>>().iter(world).count()
            };
            assert_eq!(left, 0);
        }

        #[test]
        fn nobody_rests_with_another_player_in_the_world() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80)), (9, t(100, 100))], BTreeMap::new());
            sim.take_deltas();
            sim.receive([(PlayerId(7), PlayerIntent::RestWindow)]);
            sim.update();
            assert!(sim.take_deltas().iter().all(|(_, d)| !matches!(d, LedgerDelta::RestWindow { .. } | LedgerDelta::RestRefused { .. })));
        }

        #[test]
        fn the_key_opens_a_locked_door_and_leaves_it_locked() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(89, 80))], BTreeMap::new());
            sim.take_deltas();
            let give = |sim: &mut SimApp, key: i32| {
                let world = sim.app.world_mut();
                let mut inventory = world.query_filtered::<&mut crate::Inventory, With<PlayerControlled>>().single_mut(world).unwrap();
                inventory.items = vec![crate::CarriedItem { prototype: 1, location: 0, quantity: 0, keys: vec![key], written: 0 }];
            };
            give(&mut sim, 21);
            assert_eq!(changed(&use_door(&mut sim, 7, LOCKED, 0), 7), None, "the wrong key");
            give(&mut sim, 20);
            assert_eq!(changed(&use_door(&mut sim, 7, LOCKED, 1), 7), Some(true));
            let map = sim.app.world().resource::<LoadedMap>();
            assert!(step_allowed(map, t(90, 80), t(90, 81)), "open: passable");
            assert_eq!(map.prototypes.int(map.map_object(LOCKED).unwrap(), "portal_flags"), Some(OPF_LOCKED), "still locked");
        }

        #[test]
        fn a_use_waits_for_the_pc_to_walk_there_and_a_new_move_drops_it() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let mut sim = booted(dir.path(), &[(7, t(64, 80))], BTreeMap::new());
            sim.take_deltas();
            sim.receive([(PlayerId(7), PlayerIntent::Move { seq: 1, path: (64..=70).map(|x| t(x, 80)).collect() })]);
            assert_eq!(changed(&use_door(&mut sim, 7, DOOR, 1), 7), None, "too far yet");
            let mut opened = None;
            for _ in 0..400 {
                sim.update();
                opened = opened.or(changed(&sim.take_deltas(), 7));
            }
            assert_eq!(opened, Some(true), "opened on arrival");
            // Out of range and a newer move: dropped.
            let mut sim = booted(dir.path(), &[(7, t(64, 80))], BTreeMap::new());
            sim.take_deltas();
            sim.receive([(PlayerId(7), PlayerIntent::Move { seq: 1, path: (64..=70).map(|x| t(x, 80)).collect() })]);
            use_door(&mut sim, 7, DOOR, 1);
            sim.receive([(PlayerId(7), PlayerIntent::Move { seq: 2, path: (64..=66).map(|x| t(x, 80)).collect() })]);
            let mut opened = None;
            for _ in 0..400 {
                sim.update();
                opened = opened.or(changed(&sim.take_deltas(), 7));
            }
            assert_eq!(opened, None);
        }

        #[test]
        fn saved_changes_apply_on_load_and_reach_a_joining_player() {
            let dir = tempfile::tempdir().unwrap();
            write_map(dir.path());
            let changes = BTreeMap::from([(DOOR.sector, crate::SectorDelta { portals: BTreeMap::from([(1, true)]), ..Default::default() })]);
            let mut sim = booted(dir.path(), &[(7, t(80, 80))], changes.clone());
            let deltas = sim.take_deltas();
            assert!(deltas.contains(&(PlayerId(7), LedgerDelta::MapChanges { changes })), "{deltas:?}");
            let map = sim.app.world().resource::<LoadedMap>();
            assert!(portal_open(map.prototypes.art_id(map.map_object(LOCKED).unwrap()).unwrap()), "the saved state wins over the file");
            assert!(step_allowed(map, t(90, 80), t(90, 81)), "open: passable");
        }
    }
}
