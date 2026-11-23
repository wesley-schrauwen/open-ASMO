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
    /// The game clock when saved.
    #[serde(default)]
    pub time: crate::GameTime,
    /// The calendar's first year (day 0 is 1 January of it).
    #[serde(default = "default_first_year")]
    pub first_year: u32,
    /// The number the next dropped item gets.
    #[serde(default)]
    pub next_dropped: u32,
    /// The scripts' global variables and flags and the story state ([`crate::script`]).
    #[serde(default)]
    pub scripts: crate::ScriptGlobals,
}

fn default_first_year() -> u32 {
    crate::GameStart::default().year
}

/// One player's character in the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSave {
    pub blueprint: CharacterBlueprint,
    pub position: TileLoc,
    /// What the PC carries; `None` until it first enters the world, which gives it the
    /// blueprint's starting kit ([`crate::starting_kit`]).
    #[serde(default)]
    pub inventory: Option<crate::Inventory>,
    /// The PC's script variables and flags ([`crate::script`]).
    #[serde(default)]
    pub script_state: crate::PcScriptState,
}

/// What changed in one sector since the original data. Objects are named by their place in the
/// sector's object list ([`MapObjectId`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorDelta {
    /// Portals opened (true) or closed (false), by index in the sector's list.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub portals: BTreeMap<u32, bool>,
    /// The map's own items taken off the ground, by index in the sector's mobile objects
    /// ([`crate::LoadedMap::mobiles`]).
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub taken: std::collections::BTreeSet<u32>,
    /// Items dropped on the ground, by the number the host gave each.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dropped: BTreeMap<u32, crate::GroundItem>,
    /// Piles made by dropping items onto items, by the same numbers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub piles: BTreeMap<u32, crate::Pile>,
    /// The map's containers' contents once opened (generated contents rolled), by index in the
    /// sector's mobile objects; the others hold the map's stored list.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub containers: BTreeMap<u32, crate::Inventory>,
    /// Containers shown open (frame 1): while a loot window is on them, and sticky ones after.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub opened: std::collections::BTreeSet<u32>,
    /// Doors' `portal_flags` changed by picking their locks, by index in the sector's list.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub portal_flags: BTreeMap<u32, i32>,
    /// Containers' `container_flags` changed by picking their locks, by index in the sector's
    /// mobile objects.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub container_flags: BTreeMap<u32, i32>,
}

impl SectorDelta {
    pub fn is_empty(&self) -> bool {
        self.portals.is_empty() && self.taken.is_empty() && self.dropped.is_empty() && self.piles.is_empty() && self.containers.is_empty() && self.opened.is_empty() && self.portal_flags.is_empty() && self.container_flags.is_empty()
    }
}

/// An object of a sector file, by its place in the sector's object list (the list never
/// changes: the files are the original data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MapObjectId {
    pub sector: SectorId,
    pub index: u32,
}

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
        Self {
            version: SAVE_VERSION,
            tick: 0,
            map: start.map.clone(),
            players: BTreeMap::new(),
            sector_changes: BTreeMap::new(),
            time: crate::GameTime::at_hour(start.clock.hour),
            first_year: start.clock.year,
            next_dropped: 0,
            scripts: Default::default(),
        }
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
    /// The year and hour the clock starts at (`rules/gameinit.mes`).
    pub clock: crate::GameStart,
}

impl StartMap {
    /// Reads `Rules/MapList.mes`.
    pub fn load(vfs: &Vfs) -> Result<Self, String> {
        let data = vfs.read(MAP_LIST).map_err(|e| format!("{MAP_LIST}: {e}"))?;
        let mes = MesFile::parse(&data).map_err(|e| format!("{MAP_LIST}: {e}"))?;
        let list = parse_map_list(&mes).map_err(|e| format!("{MAP_LIST}: {e}"))?;
        let start = start_map(&list).ok_or_else(|| format!("{MAP_LIST}: no START_MAP entry"))?;
        Ok(Self { map: start.name.clone(), position: start.start, clock: crate::GameStart::load(vfs) })
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
        save.players.insert(player, PlayerSave { blueprint, position: start.position, inventory: None, script_state: Default::default() });
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

/// A save found on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveListing {
    pub path: PathBuf,
    pub save: WorldSave,
    /// When the file was last written.
    pub modified: std::time::SystemTime,
}

/// Every save (`*.save`) in `dir` that reads, newest first. Files that do not read are left out
/// and returned as `path: error`.
pub fn list_saves(dir: &Path) -> (Vec<SaveListing>, Vec<String>) {
    let mut saves = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return (saves, errors) };
    for path in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "save")) {
        let modified = fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        match WorldSave::read(&path) {
            Ok(save) => saves.push(SaveListing { path, save, modified }),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    saves.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    (saves, errors)
}

/// The result of the last write, for the owner to report.
#[derive(Resource, Debug, Default)]
pub struct SaveStatus {
    pub writes: u32,
    pub last_error: Option<String>,
}

/// Writes the save whenever the world state changed this tick.
pub fn write_save(world: Res<WorldState>, file: Option<Res<SaveFile>>, tick: Res<SimTick>, clock: Res<crate::GameClock>, mut status: ResMut<SaveStatus>) {
    let (Some(save), Some(file)) = (&world.0, file) else { return };
    if !world.is_changed() {
        return;
    }
    let save = WorldSave { tick: tick.0, time: clock.now, ..save.clone() };
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
        StartMap { map: "Arcanum1-024-fixed".into(), position: TileLoc { x: 92958, y: 82592 }, clock: Default::default() }
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
        save.sector_changes.insert(SectorId::new(3, 4), SectorDelta { portals: BTreeMap::from([(7, true), (9, false)]), ..Default::default() });
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

    #[test]
    fn saves_are_listed_newest_first_and_bad_files_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WorldState::default();
        state.add_player(Some(&start()), PlayerId(7), blueprint("Virgil"));
        let save = state.0.unwrap();
        save.write(&dir.path().join("1.save")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        save.write(&dir.path().join("2.save")).unwrap();
        fs::write(dir.path().join("3.save"), b"nonsense").unwrap();
        fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let (saves, errors) = list_saves(dir.path());
        let names: Vec<_> = saves.iter().map(|s| s.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["2.save", "1.save"]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(list_saves(&dir.path().join("none")).0.is_empty());
    }
}
