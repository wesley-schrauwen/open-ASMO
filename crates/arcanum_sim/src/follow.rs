//! Followers: NPCs who join a PC through dialogue, follow it and come along on its moves.
//!
//! - **Who leads** (field 0x119 `npc_leader`, getter 0x45de00): kept here as
//!   [`crate::WorldSave::followers`], one [`Follower`] per NPC with its leader's player. A
//!   waiting follower keeps its leader (0x4aa7a0) but is out of the leader's list (0xf1), so it
//!   neither counts nor follows.
//! - **NPC flags** (`npc_flags`, field 0x118): [`WAITING`] 0x8, [`SPREAD_OUT`] 0x10 (clear: stay
//!   close), [`LEFT`] 0x20 (left or refused, the `wt` test), [`FORCED`] 0x400 (joined with
//!   `jo 1`: not counted, never refused). Changes are kept in the sector's changes
//!   ([`crate::SectorDelta::npc_flags`]).
//! - **Joining** (`jo force line`, 0x41624d → [`can_join`] 0x4ad950 → 0x45de90): refused with a
//!   `gd_npc` line by reason ([`refusal_line`], 0x413a90); else the NPC follows and the talk goes
//!   on at NPC line `line`.
//! - **Following** (0x4ac380 → 0x4346a0): farther than 4 tiles (7 spread out) from its leader it
//!   walks to it; 31 tiles or more away it is put on the leader's tile (0x4ad4d0).
//! - **Moves:** a teleport within the map takes the followers onto the same tile (0x4d3760 /
//!   0x4d39a0).

use arcanum_formats::map::SectorId;
use arcanum_formats::mes::MesFile;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::PlayerId;

/// `npc_flags` 0x8: told to wait (`wa`).
pub const WAITING: i32 = 0x8;
/// `npc_flags` 0x10: spread out (`so`); clear, stay close (`sc`).
pub const SPREAD_OUT: i32 = 0x10;
/// `npc_flags` 0x20: left the party or was refused (`wt`).
pub const LEFT: i32 = 0x20;
/// `npc_flags` 0x400: joined by force (`jo 1`): not counted against the limit.
pub const FORCED: i32 = 0x400;
/// How near a follower keeps to its leader, in tiles (0x4346a0).
pub const STAY_CLOSE_TILES: u32 = 4;
pub const SPREAD_OUT_TILES: u32 = 7;
/// From this far a follower is put on its leader's tile (0x4ad4d0: 0x1f).
pub const LEASH_TILES: u32 = 31;
/// The AI packets (table 0x5f5cb0).
pub const GAME_AI: &str = "Rules/gameai.mes";
/// The packet used when an NPC's is not in the table (builder's call: "generic AI").
pub const GENERIC_AI: u32 = 100;
/// Persuasion's skill number: a Master (rank 3) is never refused for who they are (0x4ad950).
pub const PERSUASION: usize = 11;

/// One NPC following a PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Follower {
    /// The NPC's home: its map (folder name), sector and place among that map's mobile objects
    /// ([`crate::NpcSource`] there).
    pub map: String,
    pub sector: SectorId,
    pub index: u32,
    pub leader: PlayerId,
    /// Where it is when it went with its leader to another map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub away: Option<Away>,
}

/// A follower away from its home map (owner's rule: followers change maps with their leader,
/// as PCs do; the original takes them along too, 0x4d3760).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Away {
    /// The map it is on, and the tile it arrived on.
    pub map: String,
    pub at: arcanum_formats::map::TileLoc,
    /// Its `npc_flags` there (at home they are in the home map's changes).
    pub flags: i32,
}

/// An NPC's home: map, sector, place.
pub type NpcHome = (String, SectorId, u32);

impl Follower {
    pub fn home(&self) -> NpcHome {
        (self.map.clone(), self.sector, self.index)
    }

    pub fn is(&self, home: &NpcHome) -> bool {
        self.map == home.0 && self.sector == home.1 && self.index == home.2
    }
}

impl crate::LoadedMap {
    /// The home of the NPC at `sector`, `index` of this map: its own place, or a guest's home.
    pub fn home_of(&self, sector: SectorId, index: u32) -> NpcHome {
        self.guests.get(&(sector, index)).cloned().unwrap_or_else(|| (self.name.clone().unwrap_or_default(), sector, index))
    }

    /// Where the NPC from `home` is on this map, if it is here.
    pub fn place_of(&self, home: &NpcHome) -> Option<(SectorId, u32)> {
        if let Some((&place, _)) = self.guests.iter().find(|(_, h)| *h == home) {
            return Some(place);
        }
        (self.name.as_deref() == Some(home.0.as_str()) && !self.away.contains(&(home.1, home.2))).then_some((home.1, home.2))
    }

    /// Puts this map's followers in place, after [`crate::LoadedMap::open`]: those from here who
    /// are away are not spawned at home; those away here are read from their home map's
    /// `.mob` files and added to the mobile objects on their tile, as guests.
    pub fn place_followers(&mut self, vfs: Option<&arcanum_vfs::Vfs>, followers: &[Follower]) {
        let Some(name) = self.name.clone() else { return };
        for f in followers {
            if f.map == name && f.away.is_some() {
                self.away.insert((f.sector, f.index));
            }
            let Some(away) = f.away.as_ref().filter(|a| a.map == name) else { continue };
            let Some(mut object) = self.home_mobile(vfs, &f.map, f.sector, f.index) else { continue };
            set_location(&mut object, away.at);
            object.set_int("npc_flags", away.flags);
            let list = self.mobiles.entry(away.at.sector()).or_default();
            list.push(object);
            self.guests.insert((away.at.sector(), list.len() as u32 - 1), f.home());
        }
    }

    /// NPC `index` of sector `sector` of map `map`, as its `.mob` file has it.
    fn home_mobile(&self, vfs: Option<&arcanum_vfs::Vfs>, map: &str, sector: SectorId, index: u32) -> Option<arcanum_formats::object::Object> {
        let mut home = crate::LoadedMap { prototypes: self.prototypes.clone(), ..Default::default() };
        home.read_mobiles(vfs?, map);
        home.mobiles.get(&sector)?.get(index as usize).cloned()
    }
}

/// Moves an object to `at` (field `location`).
fn set_location(object: &mut arcanum_formats::object::Object, at: arcanum_formats::map::TileLoc) {
    let packed = i64::from(at.x) | (i64::from(at.y) << 32);
    if let Some((_, v)) = object.fields.iter_mut().find(|(d, _)| d.name == "location") {
        *v = arcanum_formats::object::Value::Int64(Some(packed));
    }
}

/// A follower's `npc_flags`, wherever it is: away, on its record; at home on the loaded map,
/// there; at home on another map, in that map's kept changes.
pub fn follower_flags(f: &Follower, map: &crate::LoadedMap, other_maps: &std::collections::BTreeMap<String, std::collections::BTreeMap<SectorId, crate::SectorDelta>>) -> i32 {
    if let Some(a) = &f.away {
        return a.flags;
    }
    if map.name.as_deref() == Some(f.map.as_str()) {
        return map.npc_flags(f.sector, f.index).unwrap_or(0);
    }
    other_maps.get(&f.map).and_then(|c| c.get(&f.sector)).and_then(|d| d.npc_flags.get(&f.index)).copied().unwrap_or(0)
}

/// An NPC following a PC now (not waiting), on the NPC's entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Following {
    pub leader: PlayerId,
    pub spread_out: bool,
}

/// The follow columns of an AI packet (`Rules/gameai.mes` D6-D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiFollow {
    /// D6: at or below this reaction the NPC will not follow.
    pub min_reaction: i32,
    /// D7 / D8: how far the PC's alignment may be above / below the NPC's.
    pub above: i32,
    pub below: i32,
    /// D9: how many levels the NPC may be above the PC.
    pub levels: i32,
}

impl Default for AiFollow {
    /// The generic packet 100: `20 1001 1000 2`.
    fn default() -> Self {
        Self { min_reaction: 20, above: 1001, below: 1000, levels: 2 }
    }
}

/// Packet `packet`'s follow columns, else the generic packet's, else the defaults.
pub fn ai_follow(mes: Option<&MesFile>, packet: i32) -> AiFollow {
    let row = |id: u32| -> Option<AiFollow> {
        let numbers: Vec<i32> = mes?.get(id)?.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        Some(AiFollow { min_reaction: *numbers.get(5)?, above: *numbers.get(6)?, below: *numbers.get(7)?, levels: *numbers.get(8)? })
    };
    u32::try_from(packet).ok().and_then(row).or_else(|| row(GENERIC_AI)).unwrap_or_default()
}

/// What deciding a join reads (0x4ad950).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JoinCheck {
    pub pc_persuasion_rank: u8,
    pub pc_charisma: i32,
    pub pc_alignment: i32,
    pub pc_level: i32,
    /// Stat 15 and the followers counted now (0x45e3f0: neither waiting nor forced).
    pub max_followers: i32,
    pub followers: i32,
    /// The charisma of another PC the NPC already follows.
    pub other_leader_charisma: Option<i32>,
    /// `npc_flags` has [`FORCED`].
    pub forced: bool,
    pub reaction: i32,
    pub npc_alignment: i32,
    pub npc_level: i32,
    pub ai: AiFollow,
}

/// Why an NPC will not join (0x4ad950's codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    TooGood = 1,
    TooEvil = 2,
    Dislikes = 3,
    HasLeader = 4,
    TooMany = 5,
    NoFollowers = 6,
    TooHighLevel = 7,
}

/// The `gd_npc` block an NPC refuses with (0x413a90, table 0x413bc0).
pub fn refusal_line(r: Refusal) -> u32 {
    match r {
        Refusal::TooGood => 1600,
        Refusal::TooEvil => 1700,
        Refusal::Dislikes => 1800,
        Refusal::HasLeader => 2600,
        Refusal::TooMany => 2700,
        Refusal::NoFollowers => 2800,
        Refusal::TooHighLevel => 2900,
    }
}

/// Whether the NPC joins (0x4ad950). A Master of Persuasion skips the NPC's own conditions; a
/// forced follower skips them and the count; `ignore_count` skips the count.
pub fn can_join(c: &JoinCheck, ignore_count: bool) -> Result<(), Refusal> {
    if c.pc_persuasion_rank < 3 {
        if c.other_leader_charisma.is_some_and(|other| c.pc_charisma <= other) {
            return Err(Refusal::HasLeader);
        }
        if c.forced {
            return Ok(());
        }
        if c.reaction <= c.ai.min_reaction {
            return Err(Refusal::Dislikes);
        }
        let (npc, pc) = (c.npc_alignment, c.pc_alignment);
        if npc < pc {
            if c.ai.above < pc - npc {
                return Err(Refusal::TooGood);
            }
        } else if c.ai.below < npc - pc {
            return Err(Refusal::TooEvil);
        }
        if c.pc_level + c.ai.levels <= c.npc_level {
            return Err(Refusal::TooHighLevel);
        }
    }
    if !ignore_count {
        if c.max_followers == 0 {
            return Err(Refusal::NoFollowers);
        }
        if c.max_followers <= c.followers {
            return Err(Refusal::TooMany);
        }
    }
    Ok(())
}

/// What a conversation knows and changes about the NPC following the talking PC.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FollowTalk {
    /// The NPC follows this PC (waiting or not).
    pub mine: bool,
    /// The charisma of another PC the NPC follows.
    pub other_leader_charisma: Option<i32>,
    /// This PC's followers counted now (neither waiting nor forced).
    pub followers: i32,
    pub ai: AiFollow,
    /// The NPC's alignment and level (stats 0x13 and 0x11 of its base stats).
    pub npc_alignment: i32,
    pub npc_level: i32,
    /// Something changed: the leader or the NPC's flags.
    pub changed: bool,
    /// Joined: the NPC line the talk goes on at (`jo` / `uw`'s second number).
    pub goto: Option<i32>,
    /// Would not join: why.
    pub refused: Option<Refusal>,
}

/// How far a follower lets its leader get before walking after it.
pub fn follow_distance(spread_out: bool) -> u32 {
    if spread_out { SPREAD_OUT_TILES } else { STAY_CLOSE_TILES }
}

/// Puts [`Following`] on the NPCs that follow a PC on this map now (not waiting), and takes it
/// off the others.
pub fn sync_followers(
    mut commands: Commands,
    world: Res<crate::WorldState>,
    map: Res<crate::LoadedMap>,
    npcs: Query<(Entity, &crate::NpcSource, Option<&Following>), With<crate::Npc>>,
) {
    let (Some(save), Some(here)) = (world.0.as_ref(), map.name.as_deref()) else { return };
    let _ = here;
    for (entity, source, now) in &npcs {
        let home = map.home_of(source.sector, source.index);
        let record = save.followers.iter().find(|f| f.is(&home));
        let flags = map.npc_flags(source.sector, source.index).unwrap_or(0);
        let want = record.filter(|_| flags & WAITING == 0).map(|f| Following { leader: f.leader, spread_out: flags & SPREAD_OUT != 0 });
        match (want, now) {
            (Some(w), Some(n)) if w == *n => {}
            (Some(w), _) => {
                commands.entity(entity).insert(w);
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<Following>();
            }
            (None, None) => {}
        }
    }
}

/// Followers keep up with their leaders (0x4ac380): farther than [`follow_distance`] they walk
/// until within it; [`LEASH_TILES`] or more away they are put on the leader's tile (0x4ad4d0).
pub fn follow_leaders(
    mut commands: Commands,
    map: Res<crate::LoadedMap>,
    pcs: Query<(&crate::PlayerControlled, &crate::Position), Without<crate::Npc>>,
    mut followers: Query<(Entity, &Following, &mut crate::Position, Has<crate::Walking>), With<crate::Npc>>,
) {
    for (entity, following, mut position, walking) in &mut followers {
        let Some(leader) = pcs.iter().find(|pc| pc.0.0 == following.leader).map(|pc| pc.1.0) else { continue };
        let distance = crate::tile_distance(position.0, leader);
        if distance >= LEASH_TILES {
            position.0 = leader;
            commands.entity(entity).remove::<crate::Walking>().insert(crate::JustStopped);
            continue;
        }
        let near = follow_distance(following.spread_out);
        if walking || distance <= near {
            continue;
        }
        let Ok(mut path) = crate::pathfinding::find_path(&*map, position.0, leader) else { continue };
        if let Some(stop) = path.iter().position(|&t| crate::tile_distance(t, leader) <= near) {
            path.truncate(stop + 1);
        }
        if !path.is_empty() {
            commands.entity(entity).insert(crate::Walking { steps: path.into(), progress: 0.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check() -> JoinCheck {
        JoinCheck { pc_charisma: 8, pc_level: 1, max_followers: 2, reaction: 50, npc_level: 1, ai: AiFollow::default(), ..Default::default() }
    }

    #[test]
    fn the_generic_packet_reads_and_joins_follow_its_columns() {
        let mes = MesFile::parse(b"{100}{0 1000 1000 101 1 20 1001 1000 2 0 0 1001 50 100}{101}{0 0 0 0 0 40 10 10 0 0}").unwrap();
        assert_eq!(ai_follow(Some(&mes), 0), AiFollow::default(), "no packet 0: generic");
        let strict = ai_follow(Some(&mes), 101);
        assert_eq!(strict, AiFollow { min_reaction: 40, above: 10, below: 10, levels: 0 });
        assert_eq!(can_join(&check(), false), Ok(()));
        assert_eq!(can_join(&JoinCheck { reaction: 20, ..check() }, false), Err(Refusal::Dislikes), "at D6");
        let c = JoinCheck { ai: strict, reaction: 50, ..check() };
        assert_eq!(can_join(&JoinCheck { pc_alignment: 11, ..c }, false), Err(Refusal::TooGood));
        assert_eq!(can_join(&JoinCheck { pc_alignment: -11, ..c }, false), Err(Refusal::TooEvil));
        assert_eq!(can_join(&JoinCheck { npc_level: 1, ..c }, false), Err(Refusal::TooHighLevel), "level 1 + 0 <= 1");
        assert_eq!(can_join(&JoinCheck { followers: 2, ..check() }, false), Err(Refusal::TooMany));
        assert_eq!(can_join(&JoinCheck { followers: 2, ..check() }, true), Ok(()), "jo 1: the count is not asked");
        assert_eq!(can_join(&JoinCheck { max_followers: 0, ..check() }, false), Err(Refusal::NoFollowers));
        assert_eq!(can_join(&JoinCheck { other_leader_charisma: Some(8), ..check() }, false), Err(Refusal::HasLeader));
        assert_eq!(can_join(&JoinCheck { other_leader_charisma: Some(7), ..check() }, false), Ok(()), "more charismatic");
        assert_eq!(can_join(&JoinCheck { pc_persuasion_rank: 3, reaction: 0, ..check() }, false), Ok(()), "a Master of Persuasion");
        assert_eq!(refusal_line(Refusal::TooMany), 2700);
    }
}
