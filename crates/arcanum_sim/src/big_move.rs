//! Big moves: teleports that wait for every player (owner's rules, `plans/multiplayer.md`), and
//! the script teleport that asks for them (Arcanum.exe action 0x2c at 0x4481a1, 0x66 at
//! 0x449a85; queued by 0x4d3380, done by 0x4d3460 / 0x4d39a0).
//!
//! - **The original** queues the move and the script runs on: the move happens after the run
//!   (0x4d3330, each tick). The map is a `Rules/MapList.mes` id (5000 is the start map); the
//!   object, its followers (all of them, to the same tile) and nothing else go, onto the tile as
//!   given (no blocked-tile search), and the camera recentres on a PC.
//! - **Here** a teleport of a PC within the loaded map waits for a vote:
//!   - every player in the world gets [`LedgerDelta::MoveVote`] naming who caused it, and
//!     answers yes or no ([`PlayerIntent::Vote`](crate::PlayerIntent::Vote));
//!   - one "no" cancels it; when all said yes, the PC goes ([`LedgerDelta::Teleported`]);
//!     either way every player gets [`LedgerDelta::VoteEnded`];
//!   - players who leave drop out; there is no timeout; the clock runs and everyone can act
//!     meanwhile;
//!   - with one player in the world there is nobody to wait for: it happens at once (builder's
//!     call).
//! - The script does not wait for the vote (the original's run goes on too).
//! - **Another map** (0x4d3460 → 0x45c720): the whole party goes (every PC, as followers do in
//!   the original), to the same tile. The old map's changes are kept in the save
//!   ([`crate::WorldSave::other_maps`]) and the new map's taken out; the old map's NPCs go
//!   (`Despawn`), the new map opens around the tile and its NPCs come as its sectors load.
//!   Clients get `EnterMap`, the new map's changes and each PC's `Teleported`. Talks, barter and
//!   loot windows on the old map end.
//!
//! Not built: teleports of NPCs and other objects (reported in [`TeleportReports`]);
//! followers; fades (0x66's fade and delay, 0x67); votes kept in the save (a vote open when the
//! host stops is lost); the map's own start position and `MapList.mes` types.

use std::collections::BTreeMap;

use arcanum_formats::map::{MAP_LIST, TileLoc, parse_map_list};
use arcanum_formats::mes::MesFile;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{LedgerDelta, LoadedMap, NetId, Outbox, PlayerControlled, PlayerId, Position, ScriptObject, Session, SessionPlayer, Sheet, WorldState};

/// The script's map value for "this map" (0x4d3460 compares `id - 4999` with -1).
pub const THIS_MAP: i32 = 4998;

/// A teleport a script asked for (action 0x2c): the player whose PC triggered the script, what
/// goes, the map id and the tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptTeleport {
    pub causer: PlayerId,
    pub object: ScriptObject,
    pub map: i32,
    pub to: TileLoc,
}

/// Teleports scripts asked for this tick.
#[derive(Resource, Debug, Default)]
pub struct ScriptTeleports(pub Vec<ScriptTeleport>);

/// Teleports the host cannot do yet: what was asked.
#[derive(Resource, Debug, Default)]
pub struct TeleportReports(pub Vec<String>);

/// An open vote on a big move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub id: u32,
    pub causer: PlayerId,
    /// The PCs that go, by player.
    pub movers: Vec<PlayerId>,
    /// Another map (its folder name), or this one.
    pub map: Option<String>,
    pub to: TileLoc,
    /// Each voter's answer so far.
    pub ballots: BTreeMap<PlayerId, Option<bool>>,
}

/// The open votes.
#[derive(Resource, Debug, Default)]
pub struct BigMoves {
    next: u32,
    pub votes: Vec<Vote>,
}

/// Answers taken from the inbox: player, vote, yes.
#[derive(Resource, Debug, Default)]
pub struct VoteRequests(pub Vec<(PlayerId, u32, bool)>);

/// The map list's ids by folder name, read on first use.
#[derive(Resource, Debug, Default)]
pub struct MapIds(Option<BTreeMap<u32, String>>);

impl MapIds {
    fn name(&mut self, vfs: Option<&arcanum_vfs::Vfs>, id: u32) -> Option<&str> {
        let ids = self.0.get_or_insert_with(|| {
            let list = vfs.and_then(|v| v.read(MAP_LIST).ok()).and_then(|d| MesFile::parse(&d).ok()).and_then(|m| parse_map_list(&m).ok()).unwrap_or_default();
            list.into_iter().map(|e| (e.id, e.name)).collect()
        });
        ids.get(&id).map(String::as_str)
    }
}

/// What the vote's popup says: who wants the move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteAsk {
    pub id: u32,
    /// The causer's PC's name.
    pub causer: String,
}

type MoverQuery<'a> = (Entity, &'a NetId, &'a PlayerControlled, &'a mut Position, &'a Sheet);

/// What a move changes.
#[derive(bevy_ecs::system::SystemParam)]
pub struct MoveWorld<'w, 's> {
    commands: Commands<'w, 's>,
    session: Res<'w, Session>,
    map: ResMut<'w, LoadedMap>,
    world: ResMut<'w, WorldState>,
    outbox: ResMut<'w, Outbox>,
    populated: ResMut<'w, crate::PopulatedSectors>,
    vfs: Option<Res<'w, crate::SimVfs>>,
    pcs: Query<'w, 's, MoverQuery<'static>>,
    npcs: Query<'w, 's, (Entity, &'static NetId), With<crate::Npc>>,
    talking: Query<'w, 's, (Entity, &'static PlayerControlled), With<crate::Conversation>>,
}

/// Turns script teleports into votes, counts the answers and moves the PCs.
pub fn run_big_moves(
    mut teleports: ResMut<ScriptTeleports>,
    mut answers: ResMut<VoteRequests>,
    mut moves: ResMut<BigMoves>,
    (mut reports, mut ids): (ResMut<TeleportReports>, ResMut<MapIds>),
    mut w: MoveWorld,
) {
    let voters: Vec<PlayerId> = w.session.players.iter().filter(|(_, s)| matches!(s, SessionPlayer::InWorld(_))).map(|(&p, _)| p).collect();
    for asked in teleports.0.drain(..) {
        let ScriptObject::Pc(mover) = asked.object else {
            reports.0.push(format!("teleport of {:?} to map {} {:?}", asked.object, asked.map, asked.to));
            continue;
        };
        let name = if asked.map == THIS_MAP {
            None
        } else {
            let vfs_guard = w.vfs.as_ref().and_then(|v| v.0.read().ok());
            match u32::try_from(asked.map).ok().and_then(|id| ids.name(vfs_guard.as_deref(), id)) {
                Some(name) => Some(name.to_string()),
                None => {
                    reports.0.push(format!("teleport of {mover:?} to unknown map {} {:?}", asked.map, asked.to));
                    continue;
                }
            }
        };
        let map = name.filter(|n| Some(n.as_str()) != w.map.name.as_deref());
        // A map change takes the whole party; a move within the map, the PC.
        let movers = if map.is_some() { voters.clone() } else { vec![mover] };
        let vote = Vote { id: moves.next, causer: asked.causer, movers, map, to: asked.to, ballots: voters.iter().map(|&p| (p, None)).collect() };
        moves.next += 1;
        if voters.len() <= 1 {
            w.go(&vote);
            continue;
        }
        let causer = w.pcs.iter().find(|pc| pc.2.0 == asked.causer).map(|pc| pc.4.0.name.clone()).unwrap_or_default();
        crate::ground::broadcast(&w.session, &mut w.outbox, LedgerDelta::MoveVote { ask: VoteAsk { id: vote.id, causer } });
        moves.votes.push(vote);
    }
    for (player, id, yes) in answers.0.drain(..) {
        if let Some(ballot) = moves.votes.iter_mut().find(|v| v.id == id).and_then(|v| v.ballots.get_mut(&player)) {
            *ballot = Some(yes);
        }
    }
    let mut open = Vec::new();
    for mut vote in std::mem::take(&mut moves.votes) {
        // Leavers drop out.
        vote.ballots.retain(|p, _| voters.contains(p));
        let declined = vote.ballots.values().any(|b| *b == Some(false));
        let agreed = vote.ballots.values().all(|b| *b == Some(true));
        if declined {
            crate::ground::broadcast(&w.session, &mut w.outbox, LedgerDelta::VoteEnded { id: vote.id, passed: false });
        } else if agreed {
            w.go(&vote);
            crate::ground::broadcast(&w.session, &mut w.outbox, LedgerDelta::VoteEnded { id: vote.id, passed: true });
        } else {
            open.push(vote);
        }
    }
    moves.votes = open;
}

impl MoveWorld<'_, '_> {
    /// Does a move: another map first, then the PCs onto the tile (0x4d39a0: as given,
    /// standing, the walk dropped).
    fn go(&mut self, vote: &Vote) {
        if let Some(name) = &vote.map {
            self.switch_map(name, vote.to);
        }
        for (entity, &net_id, controlled, mut position, _) in self.pcs.iter_mut() {
            if !vote.movers.contains(&controlled.0) {
                continue;
            }
            position.0 = vote.to;
            self.commands.entity(entity).remove::<(crate::Walking, crate::PendingTalk)>();
            crate::movement::record_position(&mut self.world, controlled.0, vote.to);
            crate::ground::broadcast(&self.session, &mut self.outbox, LedgerDelta::Teleported { net_id, to: vote.to });
        }
    }

    /// Leaves the loaded map for `name`, around tile `to`.
    fn switch_map(&mut self, name: &str, to: TileLoc) {
        let Some(save) = self.world.0.as_mut() else { return };
        // The old map's NPCs go; its changes are kept.
        for (entity, &net_id) in &self.npcs {
            self.commands.entity(entity).despawn();
            crate::ground::broadcast(&self.session, &mut self.outbox, LedgerDelta::Despawn { net_id });
        }
        for (entity, controlled) in &self.talking {
            self.commands.entity(entity).remove::<crate::Conversation>();
            self.outbox.0.push((controlled.0, LedgerDelta::DialogueEnded));
        }
        for (entity, ..) in self.pcs.iter() {
            self.commands.entity(entity).remove::<(crate::barter::BarterOpen, crate::LootOpen, crate::PendingPickUp)>();
        }
        let old = std::mem::replace(&mut save.map, name.to_string());
        let changes = std::mem::take(&mut save.sector_changes);
        if !changes.is_empty() {
            save.other_maps.insert(old, changes);
        }
        save.sector_changes = save.other_maps.remove(name).unwrap_or_default();
        let vfs_guard = self.vfs.as_ref().and_then(|v| v.0.read().ok());
        let vfs = vfs_guard.as_deref();
        self.map.open(vfs, name);
        self.map.changes = save.sector_changes.clone();
        self.map.load_around(vfs, to);
        self.populated.0.clear();
        let changes = save.sector_changes.clone();
        crate::ground::broadcast(&self.session, &mut self.outbox, LedgerDelta::EnterMap { map: name.to_string() });
        if !changes.is_empty() {
            crate::ground::broadcast(&self.session, &mut self.outbox, LedgerDelta::MapChanges { changes });
        }
    }
}
