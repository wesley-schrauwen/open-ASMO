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
pub struct ScriptHost<'w> {
    scripts: ResMut<'w, Scripts>,
    started: ResMut<'w, crate::StartedDialogues>,
    teleports: ResMut<'w, crate::big_move::ScriptTeleports>,
    reports: ResMut<'w, ScriptReports>,
    rng: ResMut<'w, crate::HostRng>,
    clock: Res<'w, crate::GameClock>,
    vfs: Option<Res<'w, crate::SimVfs>>,
}

/// The world as a running script sees it.
struct HostWorld<'a> {
    library: &'a mut ScriptLibrary,
    map: &'a LoadedMap,
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
}

impl ScriptHost<'_> {
    /// Runs `object`'s script for `event`, triggered by `player`'s PC (0x441980). A changed
    /// attachment is kept and saved; the answer says whether the default still happens.
    /// Lines it shows go to their players through `outbox`.
    pub fn hook(&mut self, world: &mut WorldState, map: &mut LoadedMap, outbox: &mut crate::Outbox, object: ScriptObject, event: u32, player: PlayerId) -> HookResult {
        let mut attachment = map.attachment(object, event);
        if attachment.number == 0 {
            return HookResult::RunDefault;
        }
        let Some(save) = world.0.as_mut() else { return HookResult::RunDefault };
        let vfs_guard = self.vfs.as_ref().and_then(|v| v.0.read().ok());
        let vfs = vfs_guard.as_deref();
        let library = self.scripts.0.get_or_insert_with(|| vfs.map(ScriptLibrary::new).unwrap_or_default());
        let before = attachment;
        let number = attachment.number;
        let mut ctx = HostWorld { library, map, vfs, globals: &mut save.scripts, players: &mut save.players, rng: &mut self.rng, now: self.clock.now, messages: Vec::new(), dialogs: Vec::new(), teleports: Vec::new() };
        let invocation = Invocation { attachee: object, triggerer: ScriptObject::Pc(player), other: None, event, start_line: 0 };
        let run = crate::run_hook(&mut ctx, &mut attachment, invocation);
        for (to, text) in std::mem::take(&mut ctx.messages) {
            outbox.0.push((to, crate::LedgerDelta::ScriptMessage { text }));
        }
        self.started.0.append(&mut ctx.dialogs);
        for (object, map, to) in std::mem::take(&mut ctx.teleports) {
            self.teleports.0.push(crate::big_move::ScriptTeleport { causer: player, object, map, to });
        }
        for unsupported in run.unsupported {
            self.reports.0.push((number, unsupported));
        }
        let excess = self.reports.0.len().saturating_sub(REPORTS_KEPT);
        self.reports.0.drain(..excess);
        if attachment != before {
            map.set_attachment(object, event, attachment);
            let sector = match object {
                ScriptObject::Map(id) => id.sector,
                ScriptObject::Mobile { sector, .. } => sector,
                ScriptObject::Pc(_) => return run.result,
            };
            crate::ground::save_change(world, map, sector);
        }
        run.result
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
