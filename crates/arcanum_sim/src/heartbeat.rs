//! NPCs' heartbeat scripts (time event type 10, callback 0x4ad200; scheduled 0x4ad790 /
//! 0x4ad700).
//!
//! - **Who:** NPCs (type 0x10) on loaded sectors, switched off or not (the original keeps
//!   ticking off ones: "Gallery HB turns on when quest mentioned"), with a script at event 10
//!   (first heartbeat) or 19 (heartbeat).
//! - **When:** the first beat comes as the sector loads (0x441fc0 → 0x4ad790, 1 ms); then every
//!   [`crate::ai_delay_ms`] (0x4ad5d0: `250 + 4750 × d / 30` ms, `d` the tiles to the nearest PC,
//!   at most 30), plus 0-5000 ms at random after the first.
//! - **What:** the first beat runs event 10; unless it says "run the default", event 19 is not
//!   run that time. Event 19 runs while a PC is within 30 tiles (0x4ad420). The NPC is both
//!   triggerer and attachee. "Skip the default" holds the NPC's own thinking (wandering,
//!   [`crate::npc_idle`]) until the next beat.
//! - **Not built:** dead NPCs (nothing dies), turn-based combat's per-turn heartbeat (0x4b7010).

use std::collections::BTreeMap;

use arcanum_formats::map::SectorId;
use bevy_ecs::prelude::*;

use crate::script_host::{ScriptHost, ScriptObject};
use crate::{HookResult, LoadedMap, Outbox, PlayerControlled, PlayerId, Position, WorldState};

/// The first heartbeat event and the heartbeat event.
pub const EVENT_FIRST_HEARTBEAT: u32 = 10;
pub const EVENT_HEARTBEAT: u32 = 19;
/// How near a PC must be for event 19 to run (0x4ad420).
pub const HEARTBEAT_RANGE: u32 = 30;

/// One NPC's next beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Beat {
    pub due_ms: u64,
    pub first: bool,
}

/// The beats of the loaded NPCs with heartbeat scripts, by their place, and the time.
#[derive(Resource, Debug, Default)]
pub struct Heartbeats {
    pub beats: BTreeMap<(SectorId, u32), Beat>,
    /// NPCs looked at (with or without scripts), so each is looked at once per load.
    pub seen: std::collections::BTreeSet<(SectorId, u32)>,
    pub now_ms: u64,
    /// The map they belong to.
    pub map: Option<String>,
}

/// Runs the heartbeats that are due (0x4ad200).
#[allow(clippy::too_many_arguments)]
pub fn run_heartbeats(
    mut host: ScriptHost,
    mut beats: ResMut<Heartbeats>,
    mut world: ResMut<WorldState>,
    mut map: ResMut<LoadedMap>,
    mut outbox: ResMut<Outbox>,
    pcs: Query<(&PlayerControlled, &Position)>,
    mut npcs: Query<(&crate::NpcSource, &Position, Option<&mut crate::Standpoints>), Without<PlayerControlled>>,
) {
    beats.now_ms += (1000.0 / crate::TICK_HZ) as u64;
    let now = beats.now_ms;
    // A new map: all over again.
    if beats.map != map.name {
        beats.beats.clear();
        beats.seen.clear();
        beats.map = map.name.clone();
    }
    // Sectors gone: their beats go; sectors loaded: their NPCs' first beats.
    let loaded: Vec<SectorId> = map.sectors.keys().copied().collect();
    beats.beats.retain(|(s, _), _| loaded.contains(s));
    beats.seen.retain(|(s, _)| loaded.contains(s));
    for &sector in &loaded {
        let count = map.mobiles.get(&sector).map_or(0, Vec::len);
        for index in 0..count as u32 {
            if !beats.seen.insert((sector, index)) {
                continue;
            }
            let is_npc = map.mobiles[&sector][index as usize].object_type == arcanum_formats::object::TYPE_NPC;
            let object = ScriptObject::Mobile { sector, index };
            let scripted = [EVENT_FIRST_HEARTBEAT, EVENT_HEARTBEAT].iter().any(|&e| map.attachment(object, e).number != 0);
            if is_npc && scripted {
                beats.beats.insert((sector, index), Beat { due_ms: now + 1, first: true });
            }
        }
    }
    let pcs: Vec<(PlayerId, arcanum_formats::map::TileLoc)> = pcs.iter().map(|(p, at)| (p.0, at.0)).collect();
    let due: Vec<(SectorId, u32)> = beats.beats.iter().filter(|(_, b)| b.due_ms <= now).map(|(&k, _)| k).collect();
    for (sector, index) in due {
        let Some(beat) = beats.beats.get(&(sector, index)).copied() else { continue };
        // The dead beat no more (0x4ad200: no events; the corpse clean-up is not built).
        if map.is_dead(sector, index) {
            beats.beats.remove(&(sector, index));
            continue;
        }
        let object = ScriptObject::Mobile { sector, index };
        let at = npcs
            .iter()
            .find(|(s, ..)| s.sector == sector && s.index == index)
            .map(|(_, p, _)| p.0)
            .or_else(|| map.mobiles.get(&sector).and_then(|m| m.get(index as usize)).and_then(crate::object_location));
        let nearest = at.and_then(|at| pcs.iter().map(|&(p, pc)| (crate::tile_distance(pc, at), p)).min());
        let distance = nearest.map_or(HEARTBEAT_RANGE, |(d, _)| d.min(HEARTBEAT_RANGE));
        let mut skip_default = false;
        if let Some((_, player)) = nearest {
            let mut beat_on = true;
            if beat.first {
                let r = host.hook_by(&mut world, &mut map, &mut outbox, object, EVENT_FIRST_HEARTBEAT, player, object);
                beat_on = r == HookResult::RunDefault;
            }
            if beat_on && distance < HEARTBEAT_RANGE {
                skip_default = host.hook_by(&mut world, &mut map, &mut outbox, object, EVENT_HEARTBEAT, player, object) == HookResult::SkipDefault;
            }
        }
        let mut delay = u64::from(crate::ai_delay_ms(distance));
        if beat.first {
            delay += u64::from(host.rng().range(0, 5000) as u32);
        }
        if skip_default
            && let Some((.., Some(mut standpoints))) = npcs.iter_mut().find(|(s, ..)| s.sector == sector && s.index == index)
        {
            standpoints.next_think_ms = standpoints.next_think_ms.max(delay as u32);
        }
        if let Some(b) = beats.beats.get_mut(&(sector, index)) {
            *b = Beat { due_ms: now + delay, first: false };
        }
    }
}
