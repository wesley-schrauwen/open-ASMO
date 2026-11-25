//! Areas: the named places of the world map (`mes/gamearea.mes`, Arcanum.exe 0x4cae90 names,
//! 0x4caed0 centres) and which ones each PC knows (0x4caf50 / 0x4cafd0).
//!
//! - **The table:** `{id}{x, y, wm-x, wm-y /Name/Description/Radius:r}`; the centre is a tile of
//!   the world map, the radius (in sectors, 5 by default, -1 never) is how near the party must
//!   pass on the world map to find it.
//! - **An object's area** (0x4cb6a0): its map's `Area:` in `Rules/MapList.mes` when set; on the
//!   world map, the nearest area centre (0x4cb220) when a region test agrees (0x4be380, not
//!   decoded: here, within the area's radius); else 0, "a place unimportant".
//! - **Known areas** are kept per PC ([`crate::PcScriptState::areas`]): scripts (action 0x31),
//!   dialogue results (`mm`) and directions (`X:`) mark them; the dialogue test `ar` reads them.
//!
//! Not built: finding areas by passing near them; the world map.

use std::collections::BTreeMap;

use arcanum_formats::map::{MAP_LIST, SECTOR_TILES, TileLoc, parse_map_list};
use arcanum_formats::mes::MesFile;
use bevy_ecs::prelude::Resource;

/// The areas file.
pub const GAME_AREA: &str = "mes/gamearea.mes";
/// The world map's id in `Rules/MapList.mes`.
pub const WORLD_MAP: u32 = 5000;
/// The detection radius when an area gives none, in sectors.
pub const DEFAULT_RADIUS: i32 = 5;

/// One area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    pub id: u32,
    /// Its centre on the world map.
    pub at: TileLoc,
    pub name: String,
    pub description: String,
    /// In sectors; -1: never found by passing.
    pub radius: i32,
}

/// Parses one `gamearea.mes` entry.
pub fn parse_area(id: u32, text: &str) -> Option<Area> {
    let mut parts = text.split('/');
    let numbers: Vec<i64> = parts
        .next()?
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let (x, y) = (*numbers.first()?, *numbers.get(1)?);
    let name = parts.next().unwrap_or("").trim().to_string();
    let description = parts.next().unwrap_or("").trim().to_string();
    let radius = parts
        .find_map(|p| {
            p.trim()
                .strip_prefix("Radius:")
                .and_then(|r| r.trim().parse().ok())
        })
        .unwrap_or(DEFAULT_RADIUS);
    Some(Area {
        id,
        at: TileLoc {
            x: u32::try_from(x).ok()?,
            y: u32::try_from(y).ok()?,
        },
        name,
        description,
        radius,
    })
}

/// No areas (tests).
pub static NO_AREAS: AreaTable = AreaTable {
    loaded: true,
    areas: BTreeMap::new(),
    map_areas: BTreeMap::new(),
    world: None,
};

/// The areas and each map's area, read on first use.
#[derive(Resource, Debug, Default, Clone)]
pub struct AreaTable {
    loaded: bool,
    pub areas: BTreeMap<u32, Area>,
    /// `Rules/MapList.mes`'s `Area:` by map folder name.
    pub map_areas: BTreeMap<String, u32>,
    /// The world map's folder name.
    pub world: Option<String>,
}

impl AreaTable {
    /// A table of these areas (tests).
    pub fn of(areas: Vec<Area>) -> Self {
        Self {
            loaded: true,
            areas: areas.into_iter().map(|a| (a.id, a)).collect(),
            ..Self::default()
        }
    }

    /// Reads the files once.
    pub fn load(&mut self, vfs: Option<&arcanum_vfs::Vfs>) -> &Self {
        if !self.loaded
            && let Some(vfs) = vfs
        {
            self.loaded = true;
            if let Some(mes) = vfs
                .read(GAME_AREA)
                .ok()
                .and_then(|d| MesFile::parse(&d).ok())
            {
                self.areas = mes
                    .messages
                    .iter()
                    .filter_map(|(&id, text)| parse_area(id, text).map(|a| (id, a)))
                    .collect();
            }
            if let Some(list) = vfs
                .read(MAP_LIST)
                .ok()
                .and_then(|d| MesFile::parse(&d).ok())
                .and_then(|m| parse_map_list(&m).ok())
            {
                self.world = list
                    .iter()
                    .find(|e| e.id == WORLD_MAP)
                    .map(|e| e.name.clone());
                self.map_areas = list
                    .into_iter()
                    .filter_map(|e| Some((e.name, u32::try_from(e.area?).ok()?)))
                    .filter(|(_, a)| *a != 0)
                    .collect();
            }
        }
        self
    }

    /// The area of a place on map `map` (0x4cb6a0): the map's own, else on the world map the
    /// nearest area whose radius reaches it; 0 for none.
    pub fn area_at(&self, map: &str, at: TileLoc) -> u32 {
        if let Some(&area) = self.map_areas.get(map) {
            return area;
        }
        if self.world.as_deref() != Some(map) {
            return 0;
        }
        let distance = |a: &Area| {
            (f64::from(a.at.x) - f64::from(at.x)).hypot(f64::from(a.at.y) - f64::from(at.y))
        };
        let nearest = self
            .areas
            .values()
            .filter(|a| a.id != 0)
            .min_by(|a, b| distance(a).total_cmp(&distance(b)));
        nearest
            .filter(|a| {
                distance(a) <= f64::from(a.radius.max(DEFAULT_RADIUS) as u32 * SECTOR_TILES)
            })
            .map_or(0, |a| a.id)
    }
}

/// The direction from `from` to `to`, 0-7 clockwise from the screen's up (0x4b8d50, not read:
/// here the screen angle of the line between them, as one step's rotation, [`crate::rotation_of_step`]).
pub fn direction(from: TileLoc, to: TileLoc) -> u32 {
    let (dx, dy) = (
        f64::from(to.x) - f64::from(from.x),
        f64::from(to.y) - f64::from(from.y),
    );
    // Screen offsets (0x4b8680): x + 1 is 40 left and 20 down, y + 1 is 40 right and 20 down.
    let (sx, sy) = ((dy - dx) * 40.0, (dx + dy) * 20.0);
    let angle = sx.atan2(-sy).to_degrees().rem_euclid(360.0);
    ((angle / 45.0).round() as u32) % 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_parse_and_places_find_theirs() {
        let shrouded =
            parse_area(1, "90226,84176, 0, 0 /Shrouded Hills/A small mining town.").unwrap();
        assert_eq!(
            (shrouded.at, shrouded.name.as_str(), shrouded.radius),
            (TileLoc { x: 90226, y: 84176 }, "Shrouded Hills", 5)
        );
        let mines = parse_area(
            7,
            "83690, 48174, -5, 15 /The Black Mountain Mines/The abandoned home./Radius:-1",
        )
        .unwrap();
        assert_eq!(mines.radius, -1);
        let table = AreaTable {
            loaded: true,
            areas: [(1, shrouded), (7, mines)].into(),
            map_areas: [("Mine".to_string(), 11)].into(),
            world: Some("World".into()),
        };
        assert_eq!(
            table.area_at("Mine", TileLoc { x: 5, y: 5 }),
            11,
            "the map's own"
        );
        assert_eq!(
            table.area_at("World", TileLoc { x: 90300, y: 84100 }),
            1,
            "near Shrouded Hills"
        );
        assert_eq!(
            table.area_at("World", TileLoc { x: 10, y: 10 }),
            0,
            "nowhere"
        );
        assert_eq!(
            table.area_at("Elsewhere", TileLoc { x: 90226, y: 84176 }),
            0,
            "another map without an area"
        );
    }

    #[test]
    fn directions_follow_the_step_rotations() {
        let o = TileLoc { x: 100, y: 100 };
        for (to, r) in [
            ((90, 90), 0),
            ((90, 100), 1),
            ((90, 110), 2),
            ((100, 110), 3),
            ((110, 110), 4),
            ((110, 100), 5),
            ((110, 90), 6),
            ((100, 90), 7),
        ] {
            assert_eq!(direction(o, TileLoc { x: to.0, y: to.1 }), r, "{to:?}");
        }
    }
}
