//! The host's world: the loaded part of the map and the entities on it.
//!
//! [`boot_world`] keeps the world in step with the [`WorldState`] save: it loads the save's map,
//! spawns a PC for every player in the save who has none yet, and loads the sectors around each
//! PC. A new game and a loaded save boot the same way.
//!
//! Blocking: only the sectors' tile blocking masks are loaded. Walls, scenery and closed portals
//! block too, but object blocking is not decoded yet.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use arcanum_formats::map::{MapProperties, SectorId, TileLoc};
use arcanum_formats::sector::Sector;
use arcanum_rules::CharacterSheet;
use arcanum_vfs::Vfs;
use bevy_ecs::prelude::*;

use crate::{EntityKind, LedgerDelta, NetId, NetIds, Outbox, PlayerId, Session, SessionPlayer, WorldState};

/// Sectors loaded on each side of a PC's sector (1 = a 3 x 3 block).
pub const SECTOR_RADIUS: u32 = 1;

/// The game data the host reads maps from.
#[derive(Resource, Clone)]
pub struct SimVfs(pub Arc<RwLock<Vfs>>);

/// A tile position, for entities on the map.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position(pub TileLoc);

/// The player who controls this PC.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerControlled(pub PlayerId);

/// A character's sheet.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Sheet(pub CharacterSheet);

/// The part of the map the host has read.
#[derive(Resource, Debug, Default)]
pub struct LoadedMap {
    /// Folder name under `maps/`; `None` before a world exists.
    pub name: Option<String>,
    /// `map.prp`, if it could be read.
    pub properties: Option<MapProperties>,
    pub sectors: HashMap<SectorId, Sector>,
    /// Sectors looked for and not found (outside the map, or no file).
    pub absent: BTreeSet<SectorId>,
    /// Files that exist but could not be read, as `path: error`.
    pub errors: Vec<String>,
}

impl LoadedMap {
    /// Starts over on `name`, reading its `map.prp`.
    pub fn open(&mut self, vfs: Option<&Vfs>, name: &str) {
        *self = Self { name: Some(name.to_string()), ..Self::default() };
        let Some(vfs) = vfs else { return };
        let path = format!("maps/{name}/map.prp");
        if !vfs.exists(&path) {
            return;
        }
        match vfs.read(&path).map_err(|e| e.to_string()).and_then(|d| MapProperties::parse(&d).map_err(|e| e.to_string())) {
            Ok(p) => self.properties = Some(p),
            Err(e) => self.errors.push(format!("{path}: {e}")),
        }
    }

    /// Loads the sectors within [`SECTOR_RADIUS`] of `at` that are not loaded or known absent.
    pub fn load_around(&mut self, vfs: Option<&Vfs>, at: TileLoc) {
        let (Some(vfs), Some(name)) = (vfs, self.name.clone()) else { return };
        let centre = at.sector();
        let range = |c: u32| c.saturating_sub(SECTOR_RADIUS)..=c.saturating_add(SECTOR_RADIUS);
        for y in range(centre.y()) {
            for x in range(centre.x()) {
                let id = SectorId::new(x, y);
                if self.sectors.contains_key(&id) || self.absent.contains(&id) {
                    continue;
                }
                let path = format!("maps/{name}/{}", id.file_name());
                if !vfs.exists(&path) {
                    self.absent.insert(id);
                    continue;
                }
                match vfs.read(&path).map_err(|e| e.to_string()).and_then(|d| Sector::parse(&d).map_err(|e| e.to_string())) {
                    Ok(sector) => {
                        self.sectors.insert(id, sector);
                    }
                    Err(e) => {
                        self.errors.push(format!("{path}: {e}"));
                        self.absent.insert(id);
                    }
                }
            }
        }
    }

    /// The tile blocking mask at `at`: `None` when its sector is not loaded or has no mask. The
    /// mask alone does not decide walkability (see the module docs).
    pub fn tile_mask_blocked(&self, at: TileLoc) -> Option<bool> {
        let (x, y) = at.tile_in_sector();
        self.sectors.get(&at.sector())?.tile_mask_blocked(x, y)
    }
}

/// Boots and grows the world from the [`WorldState`]: see the module docs. Each new PC is
/// announced with a `Spawn` to every player in the session, and the new PC's player is also
/// told about the PCs already there.
#[allow(clippy::too_many_arguments)]
pub fn boot_world(
    mut commands: Commands,
    world: Res<WorldState>,
    vfs: Option<Res<SimVfs>>,
    mut map: ResMut<LoadedMap>,
    mut ids: ResMut<NetIds>,
    mut session: ResMut<Session>,
    mut outbox: ResMut<Outbox>,
    pcs: Query<(&NetId, &PlayerControlled, &Position)>,
) {
    let Some(save) = &world.0 else { return };
    let vfs_guard = vfs.as_ref().and_then(|v| v.0.read().ok());
    let vfs = vfs_guard.as_deref();
    if map.name.as_deref() != Some(save.map.as_str()) {
        map.open(vfs, &save.map);
    }
    let mut existing: Vec<(NetId, PlayerId, TileLoc)> = pcs.iter().map(|(id, p, pos)| (*id, p.0, pos.0)).collect();
    for (&player, saved) in &save.players {
        if existing.iter().any(|(_, p, _)| *p == player) {
            continue;
        }
        let net_id = ids.0.allocate();
        commands.spawn((net_id, PlayerControlled(player), Position(saved.position), Sheet(saved.blueprint.sheet.clone())));
        map.load_around(vfs, saved.position);
        session.players.insert(player, SessionPlayer::InWorld(net_id));
        // Everyone already here learns of the new PC; the new PC's player learns of everyone.
        let spawn = LedgerDelta::Spawn { net_id, kind: EntityKind::Pc { player }, position: saved.position };
        for &other in session.players.keys() {
            outbox.0.push((other, spawn.clone()));
        }
        for &(id, other, position) in &existing {
            outbox.0.push((player, LedgerDelta::Spawn { net_id: id, kind: EntityKind::Pc { player: other }, position }));
        }
        existing.push((net_id, player, saved.position));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use arcanum_formats::test_util::SectorBuilder;
    use arcanum_rules::{CharacterBlueprint, RulesData};

    use super::*;
    use crate::{PlayerSave, SimApp, WorldSave};

    const MAP: &str = "Test";

    /// A map folder with `map.prp` and a sector at (1, 1) whose tile (3, 5) is mask-blocked.
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
        let mut sector = SectorBuilder::new();
        let i = 5 * 64 + 3;
        sector.blocked[i / 32] |= 1 << (i % 32);
        std::fs::write(folder.join(SectorId::new(1, 1).file_name()), sector.build()).unwrap();
    }

    fn vfs(dir: &Path) -> SimVfs {
        let mut vfs = Vfs::new();
        vfs.mount_dir(dir);
        SimVfs(Arc::new(RwLock::new(vfs)))
    }

    fn save(players: &[(u64, &str, TileLoc)]) -> WorldSave {
        let mut save = WorldSave { map: MAP.into(), ..WorldSave::new_game(&crate::StartMap { map: MAP.into(), position: TileLoc { x: 0, y: 0 } }) };
        for &(id, name, position) in players {
            let sheet = CharacterSheet { name: name.into(), ..Default::default() };
            save.players.insert(PlayerId(id), PlayerSave { blueprint: CharacterBlueprint { sheet, gold: 0, items: vec![], prototype: None }, position });
        }
        save
    }

    fn booted(dir: &Path, save: WorldSave) -> SimApp {
        let mut sim = SimApp::new(RulesData::default());
        sim.use_manual_clock();
        sim.app.insert_resource(vfs(dir));
        sim.boot(save);
        sim.update();
        sim.update();
        sim
    }

    fn pcs(sim: &mut SimApp) -> Vec<(NetId, PlayerId, TileLoc, String)> {
        let world = sim.app.world_mut();
        let mut out: Vec<_> =
            world.query::<(&NetId, &PlayerControlled, &Position, &Sheet)>().iter(world).map(|(n, p, pos, s)| (*n, p.0, pos.0, s.0.name.clone())).collect();
        out.sort_by_key(|pc| pc.1);
        out
    }

    #[test]
    fn a_save_boots_into_pcs_and_the_sectors_around_them() {
        let dir = tempfile::tempdir().unwrap();
        write_map(dir.path());
        let at = TileLoc { x: 64 + 10, y: 64 + 20 }; // In sector (1, 1).
        let mut sim = booted(dir.path(), save(&[(7, "Virgil", at)]));
        assert_eq!(pcs(&mut sim), vec![(NetId(0), PlayerId(7), at, "Virgil".to_string())]);
        let map = sim.app.world().resource::<LoadedMap>();
        assert_eq!(map.name.as_deref(), Some(MAP));
        assert_eq!(map.properties.map(|p| (p.width, p.height)), Some((256, 256)));
        assert_eq!(map.sectors.keys().copied().collect::<Vec<_>>(), vec![SectorId::new(1, 1)]);
        assert_eq!(map.absent.len(), 8, "the other eight of the 3 x 3 have no file");
        assert!(map.errors.is_empty(), "{:?}", map.errors);
        assert_eq!(map.tile_mask_blocked(TileLoc { x: 64 + 3, y: 64 + 5 }), Some(true));
        assert_eq!(map.tile_mask_blocked(TileLoc { x: 64 + 4, y: 64 + 5 }), Some(false));
        assert_eq!(map.tile_mask_blocked(TileLoc { x: 300, y: 300 }), None, "not loaded");
        let session = sim.app.world().resource::<Session>();
        assert_eq!(session.players[&PlayerId(7)], SessionPlayer::InWorld(NetId(0)));
    }

    #[test]
    fn booting_twice_gives_the_same_world() {
        let dir = tempfile::tempdir().unwrap();
        write_map(dir.path());
        let s = save(&[(7, "Virgil", TileLoc { x: 70, y: 80 }), (9, "Lugard", TileLoc { x: 100, y: 90 })]);
        let mut a = booted(dir.path(), s.clone());
        let mut b = booted(dir.path(), s);
        assert_eq!(pcs(&mut a), pcs(&mut b));
        assert_eq!(pcs(&mut a).len(), 2);
    }

    #[test]
    fn a_pc_at_the_map_corner_loads_without_underflow() {
        let dir = tempfile::tempdir().unwrap();
        write_map(dir.path());
        let mut sim = booted(dir.path(), save(&[(1, "A", TileLoc { x: 0, y: 0 })]));
        let map = sim.app.world().resource::<LoadedMap>();
        assert_eq!(map.absent.len() + map.sectors.len(), 4, "a 2 x 2 block at the corner");
        assert_eq!(map.sectors.len(), 1, "(1, 1) is within reach");
        assert_eq!(pcs(&mut sim).len(), 1);
    }

    #[test]
    fn spawns_reach_every_player() {
        let dir = tempfile::tempdir().unwrap();
        write_map(dir.path());
        let mut sim = booted(dir.path(), save(&[(1, "A", TileLoc { x: 70, y: 70 }), (2, "B", TileLoc { x: 71, y: 70 })]));
        let deltas = sim.take_deltas();
        let spawns = |to: u64| deltas.iter().filter(|(p, d)| p.0 == to && matches!(d, LedgerDelta::Spawn { .. })).count();
        assert_eq!((spawns(1), spawns(2)), (2, 2), "{deltas:?}");
        // No PC is spawned twice on later ticks.
        sim.update();
        assert!(sim.take_deltas().is_empty());
        assert_eq!(pcs(&mut sim).len(), 2);
    }
}
