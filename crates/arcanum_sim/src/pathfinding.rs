//! Tile pathfinding: A* over the tile grid, shared by the host and the client.
//!
//! These are plain functions over a [`Walkable`] map, so the client can path its own PC at once
//! and the host can check the result with the same rules ([`step_allowed`]).
//!
//! Steps go to any of the 8 neighbouring tiles. A* costs a straight step 10 and a diagonal one
//! 14 (octile distance), so paths are short and do not zig-zag (not checked against the
//! original). A step must enter a walkable tile and cross no blocked edge; a diagonal step
//! needs both L-shaped routes round it clear of blocked edges, as in the original
//! ([`crate::blocking`]).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use arcanum_formats::map::TileLoc;

use crate::LoadedMap;

/// The most tiles A* looks at before giving up.
pub const MAX_SEARCH: usize = 20_000;

const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;

/// The 8 neighbour offsets, in the order A* tries them.
const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
    (1, -1),
];

/// Whether an entity can stand on a tile, and which tile edges it may not cross.
pub trait Walkable {
    fn walkable(&self, at: TileLoc) -> bool;

    /// Whether the edge of `at` in (odd) direction `dir` is blocked, for a diagonal step or not.
    fn edge_blocked(&self, _at: TileLoc, _dir: usize, _diagonal: bool) -> bool {
        false
    }
}

/// A tile is walkable when its sector is loaded, it is inside the map, the sector's tile mask
/// does not block it, and no object on it blocks ([`ObjectBlocking`](crate::ObjectBlocking)).
impl Walkable for LoadedMap {
    fn walkable(&self, at: TileLoc) -> bool {
        if let Some(p) = &self.properties
            && (u64::from(at.x) >= p.width || u64::from(at.y) >= p.height)
        {
            return false;
        }
        let (x, y) = at.tile_in_sector();
        self.sectors
            .get(&at.sector())
            .is_some_and(|s| s.tile_mask_blocked(x, y) != Some(true))
            && !self.blocking.tiles.contains(&at)
    }

    fn edge_blocked(&self, at: TileLoc, dir: usize, diagonal: bool) -> bool {
        self.blocking.edge_blocked(at, dir, diagonal)
    }
}

/// Why no path was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// The goal is not walkable.
    GoalBlocked,
    /// No path within [`MAX_SEARCH`] tiles.
    NoPath,
}

/// The tile at `at` moved by `(dx, dy)`, if it stays on the grid.
pub fn offset(at: TileLoc, (dx, dy): (i32, i32)) -> Option<TileLoc> {
    Some(TileLoc {
        x: at.x.checked_add_signed(dx)?,
        y: at.y.checked_add_signed(dy)?,
    })
}

/// Whether `a` and `b` are different tiles that touch (sides or corners).
pub fn adjacent(a: TileLoc, b: TileLoc) -> bool {
    a != b && a.x.abs_diff(b.x) <= 1 && a.y.abs_diff(b.y) <= 1
}

/// Whether crossing from `at` to its neighbour in odd direction `dir` is blocked by an edge of
/// either tile (0x43fe00).
fn crossing_blocked(map: &impl Walkable, at: TileLoc, dir: usize, diagonal: bool) -> bool {
    let Some(next) = offset(at, crate::DIRECTIONS[dir]) else {
        return true;
    };
    map.edge_blocked(at, dir, diagonal) || map.edge_blocked(next, (dir + 4) % 8, diagonal)
}

/// Whether one step from `from` to `to` is allowed: the tiles touch, `to` is walkable, and no
/// blocked edge is in the way. A diagonal step needs both L-shaped routes round it clear of
/// edges; the corner tiles themselves need not be walkable.
pub fn step_allowed(map: &impl Walkable, from: TileLoc, to: TileLoc) -> bool {
    if !adjacent(from, to) || !map.walkable(to) {
        return false;
    }
    let Some(dir) = crate::rotation_of_step(from, to) else {
        return false;
    };
    if dir % 2 == 1 {
        return !crossing_blocked(map, from, dir, false);
    }
    let (left, right) = ((dir + 7) % 8, (dir + 1) % 8);
    let route = |first: usize, second: usize| {
        let Some(corner) = offset(from, crate::DIRECTIONS[first]) else {
            return false;
        };
        !crossing_blocked(map, from, first, true) && !crossing_blocked(map, corner, second, true)
    };
    route(left, right) && route(right, left)
}

/// The octile distance between two tiles, in step costs.
fn estimate(a: TileLoc, b: TileLoc) -> u32 {
    let (dx, dy) = (a.x.abs_diff(b.x), a.y.abs_diff(b.y));
    DIAGONAL * dx.min(dy) + STRAIGHT * dx.abs_diff(dy)
}

/// The cheapest path from `start` to `goal`: the tiles to step on, ending with `goal` and not
/// including `start` (empty when they are the same tile). Ties are broken the same way every
/// time, so the host and the client find the same path on the same map.
pub fn find_path(
    map: &impl Walkable,
    start: TileLoc,
    goal: TileLoc,
) -> Result<Vec<TileLoc>, PathError> {
    if start == goal {
        return Ok(Vec::new());
    }
    if !map.walkable(goal) {
        return Err(PathError::GoalBlocked);
    }
    let mut open = BinaryHeap::new();
    let mut cost: HashMap<TileLoc, u32> = HashMap::from([(start, 0)]);
    let mut came_from: HashMap<TileLoc, TileLoc> = HashMap::new();
    // Lowest estimated total first, then nearest the goal, then by position.
    let h = estimate(start, goal);
    open.push(Reverse((h, h, start.y, start.x)));
    let mut searched = 0;
    while let Some(Reverse((_, _, y, x))) = open.pop() {
        let at = TileLoc { x, y };
        if at == goal {
            let mut path = vec![goal];
            let mut tile = goal;
            while let Some(&prev) = came_from.get(&tile) {
                if prev == start {
                    break;
                }
                path.push(prev);
                tile = prev;
            }
            path.reverse();
            return Ok(path);
        }
        searched += 1;
        if searched > MAX_SEARCH {
            break;
        }
        let g = cost[&at];
        for d in NEIGHBOURS {
            let Some(next) = offset(at, d) else { continue };
            if !step_allowed(map, at, next) {
                continue;
            }
            let step = if d.0 != 0 && d.1 != 0 {
                DIAGONAL
            } else {
                STRAIGHT
            };
            let new_cost = g + step;
            if cost.get(&next).is_some_and(|&c| c <= new_cost) {
                continue;
            }
            cost.insert(next, new_cost);
            came_from.insert(next, at);
            let h = estimate(next, goal);
            open.push(Reverse((new_cost + h, h, next.y, next.x)));
        }
    }
    Err(PathError::NoPath)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashSet;

    use super::*;

    /// A test grid: `#` blocks, anything else is walkable; tiles outside the rows are blocked.
    /// The fourth part holds blocked edges.
    pub(crate) struct Grid(
        pub HashSet<TileLoc>,
        pub u32,
        pub u32,
        pub crate::ObjectBlocking,
    );

    impl Grid {
        pub(crate) fn parse(rows: &[&str]) -> Self {
            let mut blocked = HashSet::new();
            for (y, row) in rows.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    if c == '#' {
                        blocked.insert(TileLoc {
                            x: x as u32,
                            y: y as u32,
                        });
                    }
                }
            }
            Self(
                blocked,
                rows[0].len() as u32,
                rows.len() as u32,
                Default::default(),
            )
        }
    }

    impl Walkable for Grid {
        fn walkable(&self, at: TileLoc) -> bool {
            at.x < self.1 && at.y < self.2 && !self.0.contains(&at)
        }

        fn edge_blocked(&self, at: TileLoc, dir: usize, diagonal: bool) -> bool {
            self.3.edge_blocked(at, dir, diagonal)
        }
    }

    pub(crate) fn t(x: u32, y: u32) -> TileLoc {
        TileLoc { x, y }
    }

    fn cost(start: TileLoc, path: &[TileLoc]) -> u32 {
        let mut prev = start;
        path.iter()
            .map(|&p| {
                let c = if p.x != prev.x && p.y != prev.y {
                    DIAGONAL
                } else {
                    STRAIGHT
                };
                prev = p;
                c
            })
            .sum()
    }

    fn assert_valid(map: &Grid, start: TileLoc, path: &[TileLoc]) {
        let mut prev = start;
        for &p in path {
            assert!(step_allowed(map, prev, p), "{prev:?} -> {p:?} in {path:?}");
            prev = p;
        }
    }

    #[test]
    fn open_ground_takes_the_diagonal_first() {
        let map = Grid::parse(&["......", "......", "......", "......"]);
        let path = find_path(&map, t(0, 0), t(5, 2)).unwrap();
        // Octile: two diagonals and three straights, cost 2 * 14 + 3 * 10.
        assert_eq!(cost(t(0, 0), &path), 58);
        assert_eq!(path.len(), 5);
        assert_eq!(path.last(), Some(&t(5, 2)));
        assert_valid(&map, t(0, 0), &path);
    }

    #[test]
    fn goes_round_a_wall_through_the_gap() {
        let map = Grid::parse(&[
            "...#...", //
            "...#...", "...#...", ".......",
        ]);
        let path = find_path(&map, t(0, 0), t(6, 0)).unwrap();
        // The only way is through (3, 3). Blocked tiles do not stop diagonals past them, so
        // (3, 3) is entered from (2, 2) and left to (4, 2): (0,0) to (2,2) is 2 diagonals (28),
        // 2 more diagonals through (3, 3) (28), then (4,2) to (6,0) is 2 diagonals (28): 84.
        assert_eq!(cost(t(0, 0), &path), 84);
        assert!(path.contains(&t(3, 3)), "{path:?}");
        assert_valid(&map, t(0, 0), &path);
    }

    #[test]
    fn diagonals_pass_blocked_tiles_but_not_wall_edges() {
        let mut map = Grid::parse(&[".#", ".."]);
        assert!(
            step_allowed(&map, t(0, 0), t(1, 1)),
            "the original checks only the tile entered"
        );
        // A wall on the edge of (0, 0) towards (1, 0) (direction 5): one L-route is closed.
        map.3.edges.insert(t(0, 0), 1 << 5);
        assert!(!step_allowed(&map, t(0, 0), t(1, 1)));
        assert_eq!(
            find_path(&map, t(0, 0), t(1, 1)).unwrap(),
            vec![t(0, 1), t(1, 1)]
        );
        // The same edge seen from the far tile blocks too.
        let mut map = Grid::parse(&["..", ".."]);
        map.3.edges.insert(t(1, 0), 1 << 1);
        assert!(
            !step_allowed(&map, t(0, 0), t(1, 0)) && !step_allowed(&map, t(1, 0), t(0, 0)),
            "both ways"
        );
        assert!(!step_allowed(&map, t(0, 1), t(1, 0)), "a diagonal round it");
        // A door opening's edge stops only diagonals.
        let mut map = Grid::parse(&["..", ".."]);
        map.3.diagonal_edges.insert(t(0, 0), 1 << 5);
        assert!(step_allowed(&map, t(0, 0), t(1, 0)));
        assert!(!step_allowed(&map, t(0, 0), t(1, 1)));
    }

    #[test]
    fn blocked_and_unreachable_goals_fail() {
        let map = Grid::parse(&["..#..", "..#..", "..#.."]);
        assert_eq!(
            find_path(&map, t(0, 0), t(2, 1)),
            Err(PathError::GoalBlocked)
        );
        assert_eq!(find_path(&map, t(0, 0), t(4, 0)), Err(PathError::NoPath));
        assert_eq!(find_path(&map, t(1, 1), t(1, 1)), Ok(vec![]));
    }

    #[test]
    fn the_same_search_gives_the_same_path() {
        let map = Grid::parse(&["..........", "....##....", "....##....", ".........."]);
        let a = find_path(&map, t(0, 2), t(9, 1)).unwrap();
        for _ in 0..5 {
            assert_eq!(find_path(&map, t(0, 2), t(9, 1)).unwrap(), a);
        }
    }

    #[test]
    fn the_grid_edge_is_not_crossed() {
        let map = Grid::parse(&[".."]);
        assert_eq!(offset(t(0, 0), (-1, 0)), None);
        assert_eq!(find_path(&map, t(1, 0), t(0, 0)).unwrap(), vec![t(0, 0)]);
    }
}
