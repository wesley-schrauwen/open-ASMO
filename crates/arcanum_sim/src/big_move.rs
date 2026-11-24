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
//!
//! Not built: teleports to other maps, of NPCs and of other objects (reported in
//! [`TeleportReports`]); followers; fades (0x66's fade and delay, 0x67); votes kept in the save
//! (a vote open when the host stops is lost).

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

/// Turns script teleports into votes, counts the answers and moves the PCs.
#[allow(clippy::too_many_arguments)]
pub fn run_big_moves(
    mut commands: Commands,
    mut teleports: ResMut<ScriptTeleports>,
    mut answers: ResMut<VoteRequests>,
    mut moves: ResMut<BigMoves>,
    (mut reports, mut ids): (ResMut<TeleportReports>, ResMut<MapIds>),
    (session, map, vfs): (Res<Session>, Res<LoadedMap>, Option<Res<crate::SimVfs>>),
    mut world: ResMut<WorldState>,
    mut outbox: ResMut<Outbox>,
    mut pcs: Query<MoverQuery>,
) {
    let voters: Vec<PlayerId> = session.players.iter().filter(|(_, s)| matches!(s, SessionPlayer::InWorld(_))).map(|(&p, _)| p).collect();
    let vfs_guard = vfs.as_ref().and_then(|v| v.0.read().ok());
    let vfs = vfs_guard.as_deref();
    for asked in teleports.0.drain(..) {
        let same_map = asked.map == THIS_MAP || (asked.map > 0 && ids.name(vfs, asked.map as u32).is_some_and(|n| Some(n) == map.name.as_deref()));
        let ScriptObject::Pc(mover) = asked.object else {
            reports.0.push(format!("teleport of {:?} to map {} {:?}", asked.object, asked.map, asked.to));
            continue;
        };
        if !same_map {
            reports.0.push(format!("teleport of {mover:?} to map {} {:?}", asked.map, asked.to));
            continue;
        }
        let vote = Vote { id: moves.next, causer: asked.causer, movers: vec![mover], to: asked.to, ballots: voters.iter().map(|&p| (p, None)).collect() };
        moves.next += 1;
        if voters.len() <= 1 {
            go(&mut commands, &vote, &mut world, &mut outbox, &session, &mut pcs);
            continue;
        }
        let causer = pcs.iter().find(|pc| pc.2.0 == asked.causer).map(|pc| pc.4.0.name.clone()).unwrap_or_default();
        crate::ground::broadcast(&session, &mut outbox, LedgerDelta::MoveVote { ask: VoteAsk { id: vote.id, causer } });
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
            crate::ground::broadcast(&session, &mut outbox, LedgerDelta::VoteEnded { id: vote.id, passed: false });
        } else if agreed {
            go(&mut commands, &vote, &mut world, &mut outbox, &session, &mut pcs);
            crate::ground::broadcast(&session, &mut outbox, LedgerDelta::VoteEnded { id: vote.id, passed: true });
        } else {
            open.push(vote);
        }
    }
    moves.votes = open;
}

/// Moves the vote's PCs onto its tile (0x4d39a0: as given, standing, the walk dropped).
fn go(commands: &mut Commands, vote: &Vote, world: &mut ResMut<WorldState>, outbox: &mut Outbox, session: &Session, pcs: &mut Query<MoverQuery>) {
    for (entity, &net_id, controlled, mut position, _) in pcs.iter_mut() {
        if !vote.movers.contains(&controlled.0) {
            continue;
        }
        position.0 = vote.to;
        commands.entity(entity).remove::<(crate::Walking, crate::PendingTalk)>();
        crate::movement::record_position(world, controlled.0, vote.to);
        crate::ground::broadcast(session, outbox, LedgerDelta::Teleported { net_id, to: vote.to });
    }
}
