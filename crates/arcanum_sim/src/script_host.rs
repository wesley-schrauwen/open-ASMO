//! The host's side of the script engine: which object a script runs for, where its attachment
//! is kept, and the world it sees ([`crate::script`] is the runner).
//!
//! - **Attachments** come from the object's `scripts_idx` (field 0x20, its own or its
//!   prototype's): a sparse array whose bitset marks the events present, one 12-byte
//!   `{flags, counters, number}` entry for each, in order ([`attachments`]). Once a script
//!   changes one (its flags, counters or number), the new one is kept in the sector's changes
//!   and saved ([`crate::SectorDelta::object_scripts`] / `mobile_scripts`).
//! - **The world:** global state is the save's [`crate::ScriptGlobals`]; a PC's variables and
//!   flags are its [`crate::PlayerSave::script_state`]; random numbers come from the
//!   [`crate::HostRng`]; day is 6:00 to 17:59. Lines a script shows (action 0x18) go to the
//!   PC's player as [`crate::LedgerDelta::ScriptMessage`], in the text for the PC's gender.
//! - **Events** wired so far: [`EVENT_USE`] on doors ([`crate::use_portals`]) and containers
//!   ([`crate::pick_up_items`]), before their default (0x428c17 runs the hook before the use).
//! - Unsupported condition and action types met in the game are kept in [`ScriptReports`].

use std::collections::BTreeMap;
use std::sync::Arc;

use arcanum_formats::object::{Object, Prototypes, Value};
use arcanum_formats::script::Script;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;

use crate::{Attachment, GroundItemId, HookResult, Invocation, LoadedMap, MapObjectId, PcScriptState, PlayerId, ScriptContext, ScriptGlobals, ScriptLibrary, WorldState};

/// The "use" event (attach point 1).
pub const EVENT_USE: u32 = 1;
/// Reports kept, newest last.
pub const REPORTS_KEPT: usize = 64;

/// An object a script runs for, or that it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptObject {
    /// A player's PC.
    Pc(PlayerId),
    /// An object of a sector file (doors, scenery).
    Map(MapObjectId),
    /// One of the map's mobile objects (containers, items on the ground, NPCs).
    Mobile { sector: arcanum_formats::map::SectorId, index: u32 },
}

impl ScriptObject {
    /// A container or a map item on the ground.
    pub fn ground(id: GroundItemId) -> Option<Self> {
        match id {
            GroundItemId::Container { sector, index } | GroundItemId::Mobile { sector, index } => Some(Self::Mobile { sector, index }),
            _ => None,
        }
    }
}

/// `object`'s attachments by event: its own `scripts_idx`, else its prototype's.
pub fn attachments(object: &Object, prototypes: &Prototypes) -> BTreeMap<u32, Attachment> {
    let array = |o: &Object| match o.field("scripts_idx") {
        Some(Value::Array(Some(a))) => Some(a.clone()),
        _ => None,
    };
    let Some(array) = array(object).or_else(|| prototypes.of(object).and_then(array)) else { return BTreeMap::new() };
    if array.elem_size != 12 {
        return BTreeMap::new();
    }
    let events = array.bitset.iter().enumerate().flat_map(|(w, &word)| (0..32).filter(move |b| word >> b & 1 != 0).map(move |b| w as u32 * 32 + b));
    let entries = array.data.as_chunks::<12>().0.iter();
    events
        .zip(entries)
        .map(|(event, e)| {
            let u = |o: usize| u32::from_le_bytes(e[o..o + 4].try_into().expect("4 bytes"));
            (event, Attachment::new(u(0), u(4), u(8)))
        })
        .collect()
}

impl LoadedMap {
    /// The attachment `object` has for `event` now: changed by a script, else from the data.
    pub fn attachment(&self, object: ScriptObject, event: u32) -> Attachment {
        let (sector, index, data) = match object {
            ScriptObject::Pc(_) => return Attachment::default(),
            ScriptObject::Map(id) => (id.sector, id.index, self.map_object(id)),
            ScriptObject::Mobile { sector, index } => (sector, index, self.mobiles.get(&sector).and_then(|m| m.get(index as usize))),
        };
        let changed = self.changes.get(&sector).and_then(|c| match object {
            ScriptObject::Map(_) => c.object_scripts.get(&index),
            _ => c.mobile_scripts.get(&index),
        });
        if let Some(a) = changed.and_then(|c| c.get(&event)) {
            return *a;
        }
        data.and_then(|o| attachments(o, &self.prototypes).get(&event).copied()).unwrap_or_default()
    }

    /// Keeps a changed attachment in the sector's changes.
    pub fn set_attachment(&mut self, object: ScriptObject, event: u32, attachment: Attachment) {
        let (sector, index) = match object {
            ScriptObject::Pc(_) => return,
            ScriptObject::Map(id) => (id.sector, id.index),
            ScriptObject::Mobile { sector, index } => (sector, index),
        };
        let changes = self.changes.entry(sector).or_default();
        let by_object = match object {
            ScriptObject::Map(_) => &mut changes.object_scripts,
            _ => &mut changes.mobile_scripts,
        };
        by_object.entry(index).or_default().insert(event, attachment);
    }
}

/// The script library, built from the game data on first use.
#[derive(Resource, Debug, Default)]
pub struct Scripts(pub Option<ScriptLibrary>);

/// Scripts that met a type the runner does not have yet: the script and what it met.
#[derive(Resource, Debug, Default)]
pub struct ScriptReports(pub Vec<(u32, crate::Unsupported)>);

/// What the host needs to run a script.
#[derive(SystemParam)]
pub struct ScriptHost<'w, 's> {
    scripts: ResMut<'w, Scripts>,
    started: ResMut<'w, crate::StartedDialogues>,
    critters: Query<'w, 's, CritterQuery<'static>>,
    rules: Res<'w, crate::SimRules>,
    reputations: ResMut<'w, crate::ReputationTable>,
    timed: ResMut<'w, TimedScripts>,
    areas: ResMut<'w, crate::areas::AreaTable>,
    changed_inventories: ResMut<'w, ScriptInventories>,
    changed_sheets: ResMut<'w, ScriptSheets>,
    teleports: ResMut<'w, crate::big_move::ScriptTeleports>,
    reports: ResMut<'w, ScriptReports>,
    rng: ResMut<'w, crate::HostRng>,
    clock: Res<'w, crate::GameClock>,
    vfs: Option<Res<'w, crate::SimVfs>>,
}

/// How far the vicinity reaches (sets 8 and 9), in tiles (builder's call: the original takes a
/// view-sized rectangle round the attachee, 0x440ff0).
pub const VICINITY: u32 = 10;

/// What the script host needs of the critters: their ids, tiles, and who they are.
type CritterQuery<'a> = (&'a crate::NetId, &'a crate::Position, Option<&'a crate::PlayerControlled>, Option<&'a crate::NpcSource>);

/// The world as a running script sees it.
struct HostWorld<'a> {
    library: &'a mut ScriptLibrary,
    map: &'a mut LoadedMap,
    /// The acting player (set 0x15).
    player: PlayerId,
    /// The critters: object, tile, id.
    critters: Vec<(ScriptObject, arcanum_formats::map::TileLoc, crate::NetId)>,
    /// Lines floated over objects.
    floats: Vec<(crate::NetId, String)>,
    /// PCs whose inventory a script changed.
    inventories: std::collections::BTreeSet<PlayerId>,
    /// PCs whose sheet a script changed.
    sheets: std::collections::BTreeSet<PlayerId>,
    /// Sectors whose changes a script touched.
    sectors: std::collections::BTreeSet<arcanum_formats::map::SectorId>,
    rules: &'a arcanum_rules::RulesData,
    reputations: &'a crate::ReputationEffects,
    areas: &'a crate::areas::AreaTable,
    /// Scripts to run later (action 0x36).
    scheduled: Vec<TimedScript>,
    vfs: Option<&'a arcanum_vfs::Vfs>,
    globals: &'a mut ScriptGlobals,
    players: &'a mut BTreeMap<PlayerId, crate::PlayerSave>,
    rng: &'a mut crate::HostRng,
    now: crate::GameTime,
    /// Lines for players' message boxes.
    messages: Vec<(PlayerId, String)>,
    /// Conversations scripts started.
    dialogs: Vec<crate::StartedDialogue>,
    /// Teleports asked for: object, map, tile.
    teleports: Vec<(ScriptObject, i32, arcanum_formats::map::TileLoc)>,
}

impl ScriptContext for HostWorld<'_> {
    type Object = ScriptObject;

    fn script(&mut self, number: u32) -> Option<Arc<Script>> {
        self.library.get(self.vfs?, number)
    }

    fn globals(&mut self) -> &mut ScriptGlobals {
        self.globals
    }

    fn pc_state(&mut self, object: ScriptObject) -> Option<&mut PcScriptState> {
        let ScriptObject::Pc(player) = object else { return None };
        self.players.get_mut(&player).map(|p| &mut p.script_state)
    }

    fn random(&mut self, low: i32, high: i32) -> i32 {
        self.rng.range(low, high)
    }

    fn now(&self) -> crate::GameTime {
        self.now
    }

    fn dialog_text(&mut self, script: u32, line: i32, reader: ScriptObject) -> Option<String> {
        let female = match reader {
            ScriptObject::Pc(player) => self.players.get(&player).is_some_and(|p| p.blueprint.sheet.gender == arcanum_rules::FEMALE),
            _ => false,
        };
        let dialog = self.library.dialog(self.vfs?, script)?;
        dialog.text(line, female).map(str::to_string)
    }

    fn show_message(&mut self, to: ScriptObject, text: String) {
        if let ScriptObject::Pc(player) = to {
            self.messages.push((player, text));
        }
    }

    fn object_int(&mut self, object: ScriptObject, field: &str) -> Option<i32> {
        let data = match object {
            ScriptObject::Pc(_) => None,
            ScriptObject::Map(id) => self.map.map_object(id),
            ScriptObject::Mobile { sector, index } => self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)),
        }?;
        self.map.prototypes.int(data, field)
    }

    fn is_npc(&mut self, object: ScriptObject) -> bool {
        matches!(object, ScriptObject::Mobile { sector, index } if self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).is_some_and(|o| o.object_type == arcanum_formats::object::TYPE_NPC))
    }

    fn start_dialog(&mut self, attachee: ScriptObject, triggerer: ScriptObject, dialog: u32, line: i32) {
        // A PC talks to the other party.
        let (pc, npc) = match (attachee, triggerer) {
            (_, ScriptObject::Pc(p)) => (p, attachee),
            (ScriptObject::Pc(p), _) => (p, triggerer),
            _ => return,
        };
        self.dialogs.push(crate::StartedDialogue { player: pc, npc, dialog, line });
    }

    fn teleport(&mut self, object: ScriptObject, map: i32, to: arcanum_formats::map::TileLoc) {
        self.teleports.push((object, map, to));
    }

    fn gold(&mut self, object: ScriptObject) -> i32 {
        self.inventory(object).map_or(0, |i| i.gold())
    }

    fn change_gold(&mut self, object: ScriptObject, n: i32) {
        let Some(mut inventory) = self.inventory(object) else { return };
        if n < 0 && inventory.gold() < -n {
            return;
        }
        inventory.change_gold(n);
        self.set_inventory(object, inventory);
    }

    fn has_item_named(&mut self, object: ScriptObject, name: i32) -> bool {
        let Some(inventory) = self.inventory(object) else { return false };
        inventory.items.iter().any(|i| self.item_name(i.prototype) == Some(name))
    }

    fn transfer_item_named(&mut self, name: i32, from: ScriptObject, to: ScriptObject) {
        let (Some(source), Some(target)) = (self.inventory(from), self.inventory(to)) else { return };
        let Some(item) = source.items.iter().find(|i| self.item_name(i.prototype) == Some(name)).cloned() else { return };
        let Some((rest, item)) = source.take_out(item.location) else { return };
        let mut target = target;
        let placed = target.insert_item(crate::CarriedItem { location: 0, ..item }, self.item_facts(), i32::MAX, 0).is_ok();
        if !placed {
            return;
        }
        self.set_inventory(from, rest);
        self.set_inventory(to, target);
    }

    fn create_item(&mut self, basic: i32, into: ScriptObject) {
        let Some(mut target) = self.inventory(into) else { return };
        let Ok(basic) = u32::try_from(basic) else { return };
        let placed = target.insert(basic + crate::BASIC_TO_PROTOTYPE, 1, self.item_facts(), i32::MAX, 0).is_ok();
        if placed {
            self.set_inventory(into, target);
        }
    }

    fn object_type(&mut self, object: ScriptObject) -> i32 {
        match object {
            ScriptObject::Pc(_) => arcanum_formats::object::TYPE_PC as i32,
            ScriptObject::Map(id) => self.map.map_object(id).map_or(0, |o| o.object_type as i32),
            ScriptObject::Mobile { sector, index } => self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).map_or(0, |o| o.object_type as i32),
        }
    }

    fn float_text(&mut self, object: ScriptObject, text: String) {
        if let Some(&(_, _, net_id)) = self.critters.iter().find(|c| c.0 == object) {
            self.floats.push((net_id, text));
        }
    }

    fn attachment(&mut self, object: ScriptObject, event: u32) -> crate::Attachment {
        self.map.attachment(object, event)
    }

    fn set_attachment(&mut self, object: ScriptObject, event: u32, attachment: crate::Attachment) {
        if self.map.attachment(object, event) == attachment {
            return;
        }
        self.map.set_attachment(object, event, attachment);
        match object {
            ScriptObject::Map(id) => {
                self.sectors.insert(id.sector);
            }
            ScriptObject::Mobile { sector, .. } => {
                self.sectors.insert(sector);
            }
            ScriptObject::Pc(_) => {}
        }
    }

    fn near(&mut self, centre: ScriptObject, kind: crate::NearKind) -> Vec<ScriptObject> {
        use arcanum_formats::object::{TYPE_CONTAINER, TYPE_PORTAL, TYPE_SCENERY};
        let Some(at) = self.location(centre) else { return Vec::new() };
        let close = |there: Option<arcanum_formats::map::TileLoc>| there.is_some_and(|t| crate::within_radius(at, t, VICINITY));
        match kind {
            crate::NearKind::Critters => self.critters.iter().filter(|c| close(Some(c.1))).map(|c| c.0).collect(),
            crate::NearKind::Containers | crate::NearKind::Items => {
                let item = |t: u32| (5..=14).contains(&t);
                let mut out = Vec::new();
                for (&sector, objects) in &self.map.mobiles {
                    for (index, o) in objects.iter().enumerate() {
                        let wanted = if kind == crate::NearKind::Containers { o.object_type == TYPE_CONTAINER } else { item(o.object_type) };
                        if wanted && close(crate::object_location(o)) {
                            out.push(ScriptObject::Mobile { sector, index: index as u32 });
                        }
                    }
                }
                out
            }
            crate::NearKind::Scenery | crate::NearKind::Portals => {
                let wanted = if kind == crate::NearKind::Scenery { TYPE_SCENERY } else { TYPE_PORTAL };
                let mut out = Vec::new();
                for (&sector, loaded) in &self.map.sectors {
                    for (index, o) in loaded.objects.iter().enumerate() {
                        if o.object_type == wanted && close(crate::object_location(o)) {
                            out.push(ScriptObject::Map(MapObjectId { sector, index: index as u32 }));
                        }
                    }
                }
                out
            }
        }
    }

    /// PCs keep their effects on the sheet. NPCs' are not kept yet (builder's call: their
    /// stats are read from their prototype's base stats, without effects).
    fn give_effect(&mut self, object: ScriptObject, id: i32, cause: i32) {
        if let (ScriptObject::Pc(p), Ok(id)) = (object, u32::try_from(id))
            && let Some(saved) = self.players.get_mut(&p)
        {
            saved.blueprint.sheet.add_effect(id, u32::try_from(cause).unwrap_or(0));
            self.sheets.insert(p);
        }
    }

    fn remove_effect(&mut self, object: ScriptObject, id: i32) {
        if let (ScriptObject::Pc(p), Ok(id)) = (object, u32::try_from(id))
            && let Some(saved) = self.players.get_mut(&p)
            && saved.blueprint.sheet.remove_effect(id)
        {
            self.sheets.insert(p);
        }
    }

    fn stat(&mut self, object: ScriptObject, stat: i32) -> i32 {
        match object {
            ScriptObject::Pc(p) => {
                let Some(sheet) = self.players.get(&p).map(|s| s.blueprint.sheet.clone()) else { return 0 };
                pc_stat(self.rules, &sheet, stat)
            }
            ScriptObject::Mobile { sector, index } => {
                let Some(o) = self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)) else { return 0 };
                let stats = o.ints("critter_stat_base_idx").or_else(|| self.map.prototypes.of(o)?.ints("critter_stat_base_idx"));
                usize::try_from(stat).ok().and_then(|i| stats?.get(i).copied()).unwrap_or(0)
            }
            ScriptObject::Map(_) => 0,
        }
    }

    fn reaction(&mut self, npc: ScriptObject, pc: ScriptObject) -> i32 {
        let (ScriptObject::Mobile { sector, index }, ScriptObject::Pc(p)) = (npc, pc) else { return 50 };
        let Some(o) = self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).filter(|o| o.object_type == arcanum_formats::object::TYPE_NPC) else { return 50 };
        let Some(saved) = self.players.get(&p) else { return 50 };
        crate::reaction_to(&saved.script_state, &crate::met_key(npc), &crate::NpcReaction::of(o, &self.map.prototypes), self.rules, &saved.blueprint.sheet, self.reputations)
    }

    fn change_reaction(&mut self, npc: ScriptObject, pc: ScriptObject, delta: i32) {
        let (ScriptObject::Mobile { .. }, ScriptObject::Pc(p)) = (npc, pc) else { return };
        if delta == 0 || !self.is_npc(npc) {
            return;
        }
        let ScriptObject::Mobile { sector, index } = npc else { return };
        let Some(base) = self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).map(|o| crate::NpcReaction::of(o, &self.map.prototypes).base) else { return };
        let key = crate::met_key(npc);
        if let Some(saved) = self.players.get_mut(&p) {
            let stored = saved.script_state.reactions.get(&key).copied().unwrap_or(base);
            saved.script_state.reactions.insert(key, stored + delta);
        }
    }

    fn distance(&mut self, a: ScriptObject, b: ScriptObject) -> i32 {
        let (Some(a), Some(b)) = (self.location(a), self.location(b)) else { return 0 };
        (f64::from(a.x) - f64::from(b.x)).hypot(f64::from(a.y) - f64::from(b.y)) as i32
    }

    fn has_met(&mut self, npc: ScriptObject, pc: ScriptObject) -> bool {
        if npc == pc {
            return true;
        }
        let ScriptObject::Pc(p) = pc else { return false };
        let key = crate::met_key(npc);
        self.players.get(&p).is_some_and(|s| s.script_state.met.contains(&key) || s.script_state.reactions.contains_key(&key))
    }

    fn knows_spell(&mut self, object: ScriptObject, spell: i32) -> bool {
        let ScriptObject::Pc(p) = object else { return false };
        let Some(sheet) = self.players.get(&p).map(|s| &s.blueprint.sheet) else { return false };
        (0..80).contains(&spell) && sheet.spells.get(spell as usize / 5).is_some_and(|&level| level > spell % 5)
    }

    fn schedule(&mut self, script: u32, line: i32, triggerer: ScriptObject, attachee: ScriptObject, delay_ms: u64) {
        self.scheduled.push(TimedScript { at: self.now.plus_ms(delay_ms), script, line, triggerer, attachee });
    }

    fn area_of(&mut self, object: ScriptObject) -> i32 {
        let (Some(at), Some(map)) = (self.location(object), self.map.name.clone()) else { return 0 };
        self.areas.area_at(&map, at) as i32
    }

    fn player(&mut self) -> Option<ScriptObject> {
        Some(ScriptObject::Pc(self.player))
    }
}

impl HostWorld<'_> {
    /// Where an object stands.
    fn location(&self, object: ScriptObject) -> Option<arcanum_formats::map::TileLoc> {
        if let Some(c) = self.critters.iter().find(|c| c.0 == object) {
            return Some(c.1);
        }
        match object {
            ScriptObject::Map(id) => crate::object_location(self.map.map_object(id)?),
            ScriptObject::Mobile { sector, index } => crate::object_location(self.map.mobiles.get(&sector)?.get(index as usize)?),
            ScriptObject::Pc(_) => None,
        }
    }

    /// An object's inventory: a PC's (its save's), an NPC's, or a container's.
    fn inventory(&self, object: ScriptObject) -> Option<crate::Inventory> {
        match object {
            ScriptObject::Pc(p) => self.players.get(&p)?.inventory.clone(),
            ScriptObject::Mobile { sector, index } => {
                let o = self.map.mobiles.get(&sector)?.get(index as usize)?;
                match o.object_type {
                    arcanum_formats::object::TYPE_NPC => self.map.npc_inventory(sector, index),
                    arcanum_formats::object::TYPE_CONTAINER => self.map.pile(GroundItemId::Container { sector, index }).map(|p| p.inventory),
                    _ => None,
                }
            }
            ScriptObject::Map(_) => None,
        }
    }

    fn set_inventory(&mut self, object: ScriptObject, inventory: crate::Inventory) {
        match object {
            ScriptObject::Pc(p) => {
                if let Some(saved) = self.players.get_mut(&p) {
                    saved.inventory = Some(inventory);
                    self.inventories.insert(p);
                }
            }
            ScriptObject::Mobile { sector, index } => {
                let kind = self.map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).map(|o| o.object_type);
                if kind == Some(arcanum_formats::object::TYPE_NPC) {
                    self.map.set_npc_inventory(sector, index, inventory);
                } else if let Some(mut pile) = self.map.pile(GroundItemId::Container { sector, index }) {
                    pile.inventory = inventory;
                    self.map.set_pile(GroundItemId::Container { sector, index }, Some(pile));
                }
                self.sectors.insert(sector);
            }
            ScriptObject::Map(_) => {}
        }
    }

    /// An item prototype's `name` (obj_f_name, field 0x16).
    fn item_name(&self, prototype: u32) -> Option<i32> {
        self.map.prototypes.get(prototype)?.int("name")
    }

    /// Item facts for placing items, with the art's sizes.
    fn item_facts(&self) -> impl Fn(u32) -> Option<crate::ItemInfo> + '_ {
        let (prototypes, art) = (self.map.prototypes.clone(), self.map.art.clone());
        let vfs = self.vfs;
        move |n| prototypes.get(n).map(|p| crate::ItemInfo::of(p, |id| crate::world::art_size(vfs, art.as_deref(), id)))
    }
}

/// A PC's effective stat (0x4b0490 order): 0-7 primary, 8-15 derived, 0x10 the aptitude, 0x11
/// the level, 0x15 unspent points, 0x19 age, 0x1a gender, 0x1b race; experience, alignment,
/// fate points, magick and tech points and poison are not kept yet (0).
pub fn pc_stat(rules: &arcanum_rules::RulesData, sheet: &arcanum_rules::CharacterSheet, stat: i32) -> i32 {
    match stat {
        0..=7 => rules.stats(sheet)[stat as usize],
        8..=15 => rules.derived(sheet).by_stat_id()[stat as usize - 8],
        0x10 => rules.aptitude(sheet),
        0x11 => sheet.level,
        0x15 => sheet.unspent,
        0x19 => sheet.age,
        0x1a => i32::from(sheet.gender),
        0x1b => i32::from(sheet.race),
        _ => 0,
    }
}

/// A script run waiting for its time (action 0x36).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedScript {
    pub at: crate::GameTime,
    pub script: u32,
    pub line: i32,
    pub triggerer: ScriptObject,
    pub attachee: ScriptObject,
}

/// Script runs waiting for their time (not in the save yet).
#[derive(Resource, Debug, Default)]
pub struct TimedScripts(pub Vec<TimedScript>);

/// Runs the scripts whose time has come ([`ScriptHost::run_due`]).
pub fn run_timed_scripts(mut scripts: ScriptHost, mut world: ResMut<WorldState>, mut map: ResMut<LoadedMap>, mut outbox: ResMut<crate::Outbox>) {
    if scripts.timed.0.is_empty() {
        return;
    }
    scripts.run_due(&mut world, &mut map, &mut outbox);
}

/// PCs whose inventory a script changed: [`sync_script_inventories`] sends them.
#[derive(Resource, Debug, Default)]
pub struct ScriptInventories(pub std::collections::BTreeSet<PlayerId>);

/// Copies inventories scripts changed from the save to the PCs and their players.
pub fn sync_script_inventories(mut changed: ResMut<ScriptInventories>, world: Res<WorldState>, acks: Res<crate::InventoryAcks>, mut outbox: ResMut<crate::Outbox>, mut pcs: Query<(&crate::NetId, &crate::PlayerControlled, &mut crate::Inventory)>) {
    if changed.0.is_empty() {
        return;
    }
    for player in std::mem::take(&mut changed.0) {
        let Some(saved) = world.0.as_ref().and_then(|w| w.players.get(&player)).and_then(|p| p.inventory.clone()) else { continue };
        let Some((&net_id, _, mut inventory)) = pcs.iter_mut().find(|pc| pc.1.0 == player) else { continue };
        *inventory = saved.clone();
        let ack = acks.0.get(&player).copied().unwrap_or(0);
        outbox.0.push((player, crate::LedgerDelta::Inventory { net_id, ack, inventory: saved }));
    }
}

/// PCs whose sheet a script changed: [`sync_script_sheets`] sends them.
#[derive(Resource, Debug, Default)]
pub struct ScriptSheets(pub std::collections::BTreeSet<PlayerId>);

/// Copies sheets scripts changed from the save to the PCs and their players; everyone gets the
/// PC's status, as effects move its maxima.
pub fn sync_script_sheets(
    mut changed: ResMut<ScriptSheets>,
    world: Res<WorldState>,
    rules: Res<crate::SimRules>,
    session: Res<crate::Session>,
    mut outbox: ResMut<crate::Outbox>,
    mut pcs: Query<(&crate::NetId, &crate::PlayerControlled, &mut crate::Sheet)>,
) {
    for player in std::mem::take(&mut changed.0) {
        let Some(saved) = world.0.as_ref().and_then(|w| w.players.get(&player)).map(|p| p.blueprint.sheet.clone()) else { continue };
        if let Some((&net_id, _, mut sheet)) = pcs.iter_mut().find(|pc| pc.1.0 == player) {
            sheet.0 = saved.clone();
            crate::ground::broadcast(&session, &mut outbox, crate::LedgerDelta::PcStatus { net_id, status: crate::pc_status(&rules.0, &saved) });
        }
        outbox.0.push((player, crate::LedgerDelta::SheetUpdate { ack: 0, sheet: saved }));
    }
}

impl ScriptHost<'_, '_> {
    /// Runs `object`'s script for `event`, triggered by `player`'s PC (0x441980). A changed
    /// attachment is kept and saved; the answer says whether the default still happens.
    /// Lines it shows go to their players through `outbox`.
    pub fn hook(&mut self, world: &mut WorldState, map: &mut LoadedMap, outbox: &mut crate::Outbox, object: ScriptObject, event: u32, player: PlayerId) -> HookResult {
        let mut attachment = map.attachment(object, event);
        if attachment.number == 0 {
            return HookResult::RunDefault;
        }
        let before = attachment;
        let invocation = Invocation { attachee: object, triggerer: ScriptObject::Pc(player), other: None, event, start_line: 0 };
        let Some(result) = self.execute(world, map, outbox, &mut attachment, invocation, player) else { return HookResult::RunDefault };
        if attachment != before {
            map.set_attachment(object, event, attachment);
            let sector = match object {
                ScriptObject::Map(id) => id.sector,
                ScriptObject::Mobile { sector, .. } => sector,
                ScriptObject::Pc(_) => return result,
            };
            crate::ground::save_change(world, map, sector);
        }
        result
    }

    /// Runs the scripts whose time has come (action 0x36): each with a record of its own, for
    /// its triggerer's player (or the first player when the triggerer is no PC).
    pub fn run_due(&mut self, world: &mut WorldState, map: &mut LoadedMap, outbox: &mut crate::Outbox) {
        let now = self.clock.now;
        let (due, waiting): (Vec<TimedScript>, Vec<TimedScript>) = std::mem::take(&mut self.timed.0).into_iter().partition(|t| t.at <= now);
        self.timed.0 = waiting;
        for timed in due {
            let player = match timed.triggerer {
                ScriptObject::Pc(p) => Some(p),
                _ => world.0.as_ref().and_then(|w| w.players.keys().next().copied()),
            };
            let Some(player) = player else { continue };
            let invocation = Invocation { attachee: timed.attachee, triggerer: timed.triggerer, other: None, event: 0, start_line: timed.line.max(0) as usize };
            self.execute(world, map, outbox, &mut crate::Attachment::new(0, 0, timed.script), invocation, player);
        }
    }

    /// Runs `attachment`'s script for `invocation` with `player` acting, and delivers what it
    /// did. `None` without a world.
    fn execute(&mut self, world: &mut WorldState, map: &mut LoadedMap, outbox: &mut crate::Outbox, attachment: &mut crate::Attachment, invocation: Invocation<ScriptObject>, player: PlayerId) -> Option<HookResult> {
        let save = world.0.as_mut()?;
        let vfs_guard = self.vfs.as_ref().and_then(|v| v.0.read().ok());
        let vfs = vfs_guard.as_deref();
        let library = self.scripts.0.get_or_insert_with(|| vfs.map(ScriptLibrary::new).unwrap_or_default());
        let number = attachment.number;
        let critters = self
            .critters
            .iter()
            .filter_map(|(&net_id, position, pc, npc)| {
                let object = match (pc, npc) {
                    (Some(pc), _) => ScriptObject::Pc(pc.0),
                    (_, Some(npc)) => npc.object(),
                    _ => return None,
                };
                Some((object, position.0, net_id))
            })
            .collect();
        let reputations = &*self.reputations.0.get_or_insert_with(|| {
            vfs.and_then(|v| v.read(crate::GAME_REP).ok()).and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok()).map(|m| crate::ReputationEffects::parse(&m)).unwrap_or_default()
        });
        let areas = self.areas.load(vfs);
        let mut ctx = HostWorld {
            areas,
            rules: &self.rules.0,
            reputations,
            scheduled: Vec::new(),
            library,
            map,
            player,
            critters,
            floats: Vec::new(),
            inventories: Default::default(),
            sheets: Default::default(),
            sectors: Default::default(),
            vfs,
            globals: &mut save.scripts,
            players: &mut save.players,
            rng: &mut self.rng,
            now: self.clock.now,
            messages: Vec::new(),
            dialogs: Vec::new(),
            teleports: Vec::new(),
        };
        let run = crate::run_hook(&mut ctx, attachment, invocation);
        for (to, text) in std::mem::take(&mut ctx.messages) {
            outbox.0.push((to, crate::LedgerDelta::ScriptMessage { text }));
        }
        self.started.0.append(&mut ctx.dialogs);
        // Floats go to everyone in the world (the original to those who can see it).
        let players: Vec<PlayerId> = ctx.players.keys().copied().collect();
        for (speaker, text) in std::mem::take(&mut ctx.floats) {
            for &to in &players {
                outbox.0.push((to, crate::LedgerDelta::FloatText { speaker, text: text.clone(), colour: crate::FloatColour::White, voice: None }));
            }
        }
        self.changed_inventories.0.append(&mut ctx.inventories);
        self.changed_sheets.0.append(&mut ctx.sheets);
        self.timed.0.append(&mut ctx.scheduled);
        let touched = std::mem::take(&mut ctx.sectors);
        for (object, map, to) in std::mem::take(&mut ctx.teleports) {
            self.teleports.0.push(crate::big_move::ScriptTeleport { causer: player, object, map, to });
        }
        for unsupported in run.unsupported {
            self.reports.0.push((number, unsupported));
        }
        let excess = self.reports.0.len().saturating_sub(REPORTS_KEPT);
        self.reports.0.drain(..excess);
        for sector in touched {
            crate::ground::save_change(world, map, sector);
        }
        Some(run.result)
    }

    /// Puts a script in the library by hand (tests).
    pub fn insert(&mut self, number: u32, script: Script) {
        self.scripts.0.get_or_insert_with(ScriptLibrary::default).insert(number, script);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcanum_formats::object::{Array, TYPE_PORTAL};
    use arcanum_formats::test_util::ObjectBuilder;

    trait Instance {
        fn build_instance(&self) -> Object;
    }

    impl Instance for ObjectBuilder {
        fn build_instance(&self) -> Object {
            Object::parse_instance(&self.build()).unwrap()
        }
    }

    #[test]
    fn attachments_are_a_sparse_array_by_event() {
        // Events 1 and 9: script 1274 (flags 3, counters 1, 2, 0, 0) and script 1823.
        let mut data = Vec::new();
        for v in [3u32, 0x0201, 1274, 0, 0, 1823] {
            data.extend(v.to_le_bytes());
        }
        let array = Array { elem_size: 12, id: 1, data, bitset: vec![1 << 1 | 1 << 9, 0] };
        let object = ObjectBuilder::new(TYPE_PORTAL).field("scripts_idx", Value::Array(Some(array))).build_instance();
        let found = attachments(&object, &Prototypes::default());
        assert_eq!(found.len(), 2);
        assert_eq!(found[&1], Attachment { flags: 3, counters: [1, 2, 0, 0], number: 1274 });
        assert_eq!(found[&9].number, 1823);
        assert!(attachments(&ObjectBuilder::new(TYPE_PORTAL).build_instance(), &Prototypes::default()).is_empty());
    }
}
