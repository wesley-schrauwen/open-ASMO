//! What the map's objects block, as the original's walking does (pathfinder 0x41f3c0, tile
//! check 0x4d7110, edge check 0x43fe00; see `NOTES.md` → "Blocking").
//!
//! - **Tiles.** Scenery, containers, projectiles and traps block the tile they stand on, unless
//!   flagged `NO_BLOCK` (0x400). Critters do not block out of combat (the original ignores them
//!   unless the mover is in combat mode, which is also the owner's rule).
//! - **Edges.** Walls and portals block the edge of their tile that their rotation faces (an even
//!   rotation counts as the next odd one), for crossings either way. A step to a side neighbour
//!   crosses one edge. A diagonal step needs both L-shaped routes round it clear of edges; the
//!   corner tiles themselves are not checked.
//! - **Door openings.** Wall pieces 10, 13, 14 and 17-19 (`w3a`, `w4a`, `w4b`, `w5a`-`w5c`) block
//!   only diagonal steps; pieces 22, 25, 26 and 29-32 (`d3a` ... `d6d`) never block. Most of them
//!   share their edge with a portal, which decides.
//! - **Portals.** An open portal (art bit 0x200, or a frame in bits 14-18) does not block. A
//!   closed one blocks if jammed (0x2; windows), magically held (0x4) or locked
//!   ([`crate::doors::locked`]); otherwise the PC walks through, as the original opens it on the way.
//! - Objects destroyed (0x1) or off (0x2) block nothing; nor do walls under `PASSWALLED` (spell
//!   flag 0x100000).
//!
//! Not reproduced: tile art flags (the original also blocks some ground tiles, probably water,
//! from per-tile tables not decoded yet), the lock and key check (0x4aeca0: any locked door
//! blocks), opening doors (their state never changes), and NPCs that cannot open doors.

use std::collections::{HashMap, HashSet};

use arcanum_formats::art_id::{ArtId, ArtKind};
use arcanum_formats::map::TileLoc;
use arcanum_formats::object::{
    Object, Prototypes, TYPE_CONTAINER, TYPE_PORTAL, TYPE_PROJECTILE, TYPE_SCENERY, TYPE_TRAP,
    TYPE_WALL, Value,
};

/// `flags`: destroyed.
pub const OF_DESTROYED: i32 = 0x1;
/// `flags`: off.
pub const OF_OFF: i32 = 0x2;
/// `flags`: carried, or inside a container.
pub const OF_INVENTORY: i32 = 0x1000;
/// `flags`: does not block.
pub const OF_NO_BLOCK: i32 = 0x400;
/// `spell_flags`: walls do not block (Passwall).
pub const OSF_PASSWALLED: i32 = 0x10_0000;
/// `portal_flags`.
pub const OPF_LOCKED: i32 = 0x1;
pub const OPF_JAMMED: i32 = 0x2;
pub const OPF_MAGICALLY_HELD: i32 = 0x4;

/// Wall pieces of door openings that block only diagonal steps.
const DIAGONAL_ONLY_PIECES: [u32; 6] = [10, 13, 14, 17, 18, 19];
/// Wall pieces of door openings that never block.
const OPEN_PIECES: [u32; 7] = [22, 25, 26, 29, 30, 31, 32];

/// Tile steps for each direction (0x4b8ff0), the same numbering as critter rotations
/// ([`rotation_of_step`](crate::rotation_of_step)).
pub const DIRECTIONS: [(i32, i32); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
    (1, 0),
    (1, -1),
    (0, -1),
];

/// What the map's objects block (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectBlocking {
    /// Tiles a step may not enter.
    pub tiles: HashSet<TileLoc>,
    /// Per tile, the edges (bit `dir` for odd directions) blocked for every step.
    pub edges: HashMap<TileLoc, u8>,
    /// Per tile, the edges blocked only for diagonal steps.
    pub diagonal_edges: HashMap<TileLoc, u8>,
}

/// How one object blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocks {
    Nothing,
    Tile,
    /// An edge, by direction.
    Edge(usize),
    DiagonalEdge(usize),
}

/// The edge an art id's rotation faces: an even rotation counts as the next odd one.
fn edge_of(art: ArtId) -> usize {
    let r = ((art.0 >> 11) & 7) as usize;
    if r.is_multiple_of(2) { r + 1 } else { r }
}

/// How `object` blocks, its missing fields taken from its prototype.
/// `day` says whether day locks hold ([`crate::lock_day`]).
pub fn object_blocks(object: &Object, prototypes: &Prototypes, day: bool) -> Blocks {
    let int = |name| prototypes.int(object, name).unwrap_or(0);
    if int("flags") & (OF_DESTROYED | OF_OFF) != 0 {
        return Blocks::Nothing;
    }
    match object.object_type {
        TYPE_WALL => {
            let Some(art) = prototypes.art_id(object) else {
                return Blocks::Nothing;
            };
            let piece = if art.kind() == ArtKind::Wall {
                (art.0 >> 14) & 0x3f
            } else {
                u32::MAX
            };
            if OPEN_PIECES.contains(&piece) || int("spell_flags") & OSF_PASSWALLED != 0 {
                Blocks::Nothing
            } else if DIAGONAL_ONLY_PIECES.contains(&piece) {
                Blocks::DiagonalEdge(edge_of(art))
            } else {
                Blocks::Edge(edge_of(art))
            }
        }
        TYPE_PORTAL => {
            let Some(art) = prototypes.art_id(object) else {
                return Blocks::Nothing;
            };
            let open = art.0 & 0x200 != 0 || (art.0 >> 14) & 0x1f != 0;
            let flags = int("portal_flags");
            let barred =
                flags & (OPF_JAMMED | OPF_MAGICALLY_HELD) != 0 || crate::doors::locked(flags, day);
            if !open && barred {
                Blocks::Edge(edge_of(art))
            } else {
                Blocks::Nothing
            }
        }
        TYPE_CONTAINER | TYPE_SCENERY | TYPE_PROJECTILE | TYPE_TRAP
            if int("flags") & OF_NO_BLOCK == 0 =>
        {
            Blocks::Tile
        }
        _ => Blocks::Nothing,
    }
}

/// An object's `location` field.
pub fn object_location(object: &Object) -> Option<TileLoc> {
    match object.field("location")? {
        Value::Int64(Some(packed)) => Some(TileLoc::from_packed(*packed)),
        _ => None,
    }
}

impl ObjectBlocking {
    /// Adds what `objects` block.
    pub fn add<'a>(
        &mut self,
        objects: impl IntoIterator<Item = &'a Object>,
        prototypes: &Prototypes,
        day: bool,
    ) {
        for object in objects {
            let Some(at) = object_location(object) else {
                continue;
            };
            match object_blocks(object, prototypes, day) {
                Blocks::Nothing => {}
                Blocks::Tile => {
                    self.tiles.insert(at);
                }
                Blocks::Edge(dir) => *self.edges.entry(at).or_default() |= 1 << dir,
                Blocks::DiagonalEdge(dir) => {
                    *self.diagonal_edges.entry(at).or_default() |= 1 << dir
                }
            }
        }
    }

    /// Forgets what the objects of sector `id` block.
    pub fn remove_sector(&mut self, id: arcanum_formats::map::SectorId) {
        self.tiles.retain(|t| t.sector() != id);
        self.edges.retain(|t, _| t.sector() != id);
        self.diagonal_edges.retain(|t, _| t.sector() != id);
    }

    /// Whether an object blocks the edge of `at` in direction `dir`, for a diagonal step or not.
    pub fn edge_blocked(&self, at: TileLoc, dir: usize, diagonal: bool) -> bool {
        let has = |m: &HashMap<TileLoc, u8>| m.get(&at).is_some_and(|e| e >> dir & 1 == 1);
        has(&self.edges) || (diagonal && has(&self.diagonal_edges))
    }
}

#[cfg(test)]
mod tests {
    use arcanum_formats::test_util::{ObjectBuilder, prototype_id};

    use super::*;

    fn build(b: ObjectBuilder) -> Object {
        Object::parse_instance(&b.build()).unwrap()
    }

    fn wall(piece: u32, rotation: u32) -> Object {
        let art = (1 << 28) | (piece << 14) | (rotation << 11);
        build(ObjectBuilder::new(TYPE_WALL).field("current_aid", Value::Int32(art as i32)))
    }

    fn portal(art_bits: u32, flags: i32) -> Object {
        let art = (3 << 28) | (3 << 11) | art_bits;
        build(
            ObjectBuilder::new(TYPE_PORTAL)
                .field("current_aid", Value::Int32(art as i32))
                .field("portal_flags", Value::Int32(flags)),
        )
    }

    #[test]
    fn walls_block_the_edge_they_face() {
        let p = Prototypes::default();
        assert_eq!(object_blocks(&wall(0, 3), &p, true), Blocks::Edge(3));
        assert_eq!(
            object_blocks(&wall(0, 2), &p, true),
            Blocks::Edge(3),
            "even rotations count as the next odd one"
        );
        assert_eq!(
            object_blocks(&wall(10, 5), &p, true),
            Blocks::DiagonalEdge(5),
            "w3a: a door opening"
        );
        assert_eq!(
            object_blocks(&wall(22, 5), &p, true),
            Blocks::Nothing,
            "d3a"
        );
    }

    #[test]
    fn closed_portals_block_only_when_barred() {
        let p = Prototypes::default();
        assert_eq!(
            object_blocks(&portal(0, 0), &p, true),
            Blocks::Nothing,
            "closed: the PC opens it"
        );
        assert_eq!(
            object_blocks(&portal(0, OPF_LOCKED), &p, true),
            Blocks::Edge(3)
        );
        assert_eq!(
            object_blocks(&portal(0, OPF_JAMMED), &p, true),
            Blocks::Edge(3)
        );
        assert_eq!(
            object_blocks(&portal(0x200, OPF_LOCKED), &p, true),
            Blocks::Nothing,
            "open"
        );
        assert_eq!(
            object_blocks(&portal(2 << 14, OPF_LOCKED), &p, true),
            Blocks::Nothing,
            "open (a frame)"
        );
    }

    #[test]
    fn scenery_blocks_its_tile_unless_flagged_and_fields_come_from_the_prototype() {
        let mut p = Prototypes::default();
        let scenery = |flags: Option<i32>| {
            let mut b = ObjectBuilder::new(TYPE_SCENERY);
            b.prototype = prototype_id(4001);
            if let Some(f) = flags {
                b = b.field("flags", Value::Int32(f));
            }
            build(b)
        };
        assert_eq!(object_blocks(&scenery(None), &p, true), Blocks::Tile);
        assert_eq!(
            object_blocks(&scenery(Some(OF_NO_BLOCK)), &p, true),
            Blocks::Nothing
        );
        assert_eq!(
            object_blocks(&scenery(Some(OF_OFF)), &p, true),
            Blocks::Nothing
        );
        // A prototype that does not block, and an instance that does not say.
        let mut proto = ObjectBuilder::new(TYPE_SCENERY).field("flags", Value::Int32(OF_NO_BLOCK));
        proto.id = prototype_id(4001);
        assert!(p.insert(build(proto)));
        assert_eq!(object_blocks(&scenery(None), &p, true), Blocks::Nothing);
        assert_eq!(
            object_blocks(&scenery(Some(0)), &p, true),
            Blocks::Tile,
            "the instance's own value wins"
        );
    }
}
