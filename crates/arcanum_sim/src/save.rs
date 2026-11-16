//! The session's saved state. The same [`WorldSave`] is the save file, what Load Game reads, and
//! the snapshot a joining player receives.
//!
//! A save holds references into the original data (map names, prototype numbers) plus what has
//! changed, never copies of original assets. On a new game nothing has changed: the save is the
//! start map and the players' characters.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use arcanum_formats::map::{MAP_LIST, SectorId, TileLoc, parse_map_list, start_map};
use arcanum_formats::mes::MesFile;
use arcanum_rules::CharacterBlueprint;
use arcanum_vfs::Vfs;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{PlayerId, SimTick, Tick};

/// Bumped when the format changes incompatibly.
pub const SAVE_VERSION: u32 = 1;

/// A saved session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldSave {
    pub version: u32,
    pub tick: Tick,
    /// Folder name under `maps/`.
    pub map: String,
    pub players: BTreeMap<PlayerId, PlayerSave>,
    /// Changes to sectors since the original data (none on a new game).
    pub sector_changes: BTreeMap<SectorId, SectorDelta>,
}

/// One player's character in the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSave {
    pub blueprint: CharacterBlueprint,
    pub position: TileLoc,
}

/// What changed in one sector. Empty until objects can change (moved, taken, killed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorDelta {}

/// Why a save could not be read or written.
#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
    Format(serde_json::Error),
    /// Written by an incompatible version.
    Version(u32),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(e) => write!(f, "not a save: {e}"),
            Self::Version(v) => write!(f, "save version {v}, expected {SAVE_VERSION}"),
        }
    }
}

impl std::error::Error for SaveError {}

impl WorldSave {
    /// A new game on `start`, with no players yet.
    pub fn new_game(start: &StartMap) -> Self {
        Self { version: SAVE_VERSION, tick: 0, map: start.map.clone(), players: BTreeMap::new(), sector_changes: BTreeMap::new() }
    }

    /// The encoded save. JSON for now; the one place to change the format.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("a WorldSave always serialises")
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, SaveError> {
        let save: Self = serde_json::from_slice(data).map_err(SaveError::Format)?;
        if save.version != SAVE_VERSION {
            return Err(SaveError::Version(save.version));
        }
        Ok(save)
    }

    pub fn write(&self, path: &Path) -> Result<(), SaveError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(SaveError::Io)?;
        }
        fs::write(path, self.to_bytes()).map_err(SaveError::Io)
    }

    pub fn read(path: &Path) -> Result<Self, SaveError> {
        Self::from_bytes(&fs::read(path).map_err(SaveError::Io)?)
    }
}

/// Where a new game starts: the map list's `START_MAP` entry.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct StartMap {
    pub map: String,
    pub position: TileLoc,
}

impl StartMap {
    /// Reads `Rules/MapList.mes`.
    pub fn load(vfs: &Vfs) -> Result<Self, String> {
        let data = vfs.read(MAP_LIST).map_err(|e| format!("{MAP_LIST}: {e}"))?;
        let mes = MesFile::parse(&data).map_err(|e| format!("{MAP_LIST}: {e}"))?;
        let list = parse_map_list(&mes).map_err(|e| format!("{MAP_LIST}: {e}"))?;
        let start = start_map(&list).ok_or_else(|| format!("{MAP_LIST}: no START_MAP entry"))?;
        Ok(Self { map: start.name.clone(), position: start.start })
    }
}

/// The live session state that gets saved; `None` until the first character is accepted.
#[derive(Resource, Debug, Default)]
pub struct WorldState(pub Option<WorldSave>);

impl WorldState {
    /// Puts an accepted character into the world at the start position, starting the world on
    /// the start map if this is the first. Without a [`StartMap`] there is no world to join.
    pub fn add_player(&mut self, start: Option<&StartMap>, player: PlayerId, blueprint: CharacterBlueprint) -> bool {
        let Some(start) = start else { return false };
        let save = self.0.get_or_insert_with(|| WorldSave::new_game(start));
        save.players.insert(player, PlayerSave { blueprint, position: start.position });
        true
    }
}

/// Where this host writes its save. Without it nothing is written.
#[derive(Resource, Debug, Clone)]
pub struct SaveFile(pub PathBuf);

impl SaveFile {
    /// `<dir>/<session>.save`.
    pub fn in_dir(dir: &Path, session: u64) -> Self {
        Self(dir.join(format!("{session}.save")))
    }
}

/// The result of the last write, for the owner to report.
#[derive(Resource, Debug, Default)]
pub struct SaveStatus {
    pub writes: u32,
    pub last_error: Option<String>,
}

/// Writes the save whenever the world state changed this tick.
pub fn write_save(world: Res<WorldState>, file: Option<Res<SaveFile>>, tick: Res<SimTick>, mut status: ResMut<SaveStatus>) {
    let (Some(save), Some(file)) = (&world.0, file) else { return };
    if !world.is_changed() {
        return;
    }
    let save = WorldSave { tick: tick.0, ..save.clone() };
    match save.write(&file.0) {
        Ok(()) => {
            status.writes += 1;
            status.last_error = None;
        }
        Err(e) => status.last_error = Some(format!("{}: {e}", file.0.display())),
    }
}

#[cfg(test)]
mod tests {
    use arcanum_rules::CharacterSheet;

    use super::*;

    fn start() -> StartMap {
        StartMap { map: "Arcanum1-024-fixed".into(), position: TileLoc { x: 92958, y: 82592 } }
    }

    fn blueprint(name: &str) -> CharacterBlueprint {
        CharacterBlueprint { sheet: CharacterSheet { name: name.into(), ..Default::default() }, gold: 3, items: vec![1, 2], prototype: None }
    }

    #[test]
    fn saves_round_trip() {
        let mut state = WorldState::default();
        assert!(state.add_player(Some(&start()), PlayerId(7), blueprint("Virgil")));
        let mut save = state.0.unwrap();
        save.tick = 42;
        save.sector_changes.insert(SectorId::new(3, 4), SectorDelta::default());
        assert_eq!(WorldSave::from_bytes(&save.to_bytes()).unwrap(), save);
    }

    #[test]
    fn other_versions_are_refused() {
        let mut save = WorldSave::new_game(&start());
        save.version = SAVE_VERSION + 1;
        assert!(matches!(WorldSave::from_bytes(&save.to_bytes()), Err(SaveError::Version(_))));
        assert!(matches!(WorldSave::from_bytes(b"nonsense"), Err(SaveError::Format(_))));
    }

    #[test]
    fn players_join_at_the_start() {
        let mut state = WorldState::default();
        assert!(!state.add_player(None, PlayerId(1), blueprint("A")), "no start map, no world");
        state.add_player(Some(&start()), PlayerId(1), blueprint("A"));
        state.add_player(Some(&start()), PlayerId(2), blueprint("B"));
        let save = state.0.unwrap();
        assert_eq!(save.map, "Arcanum1-024-fixed");
        assert_eq!(save.players.len(), 2);
        assert!(save.players.values().all(|p| p.position == start().position));
    }

    #[test]
    fn reads_and_writes_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = SaveFile::in_dir(&dir.path().join("saves"), 9);
        let save = WorldSave::new_game(&start());
        save.write(&file.0).unwrap();
        assert_eq!(WorldSave::read(&file.0).unwrap(), save);
        assert!(file.0.ends_with("saves/9.save"));
    }
}
