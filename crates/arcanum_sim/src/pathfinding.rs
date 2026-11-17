//! Tile pathfinding: A* over the tile grid, shared by the host and the client.
//!
//! These are plain functions over a [`Walkable`] map, so the client can path its own PC at once
//! and the host can check the result with the same rules ([`step_allowed`]).
//!
//! Steps go to any of the 8 neighbouring tiles. A* costs a straight step 10 and a diagonal one
//! 14 (octile distance), so paths are short and do not zig-zag; a diagonal step is refused when
//! either tile it cuts past is not walkable. Neither rule is checked against the original yet.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use arcanum_formats::map::TileLoc;

use crate::LoadedMap;

/// The most tiles A* looks at before giving up.
pub const MAX_SEARCH: usize = 20_000;

const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;

/// The 8 neighbour offsets, in the order A* tries them.
const NEIGHBOURS: [(i32, i32); 8] = [(1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (-1, 1), (-1, -1), (1, -1)];

/// Whether an entity can stand on a tile.
pub trait Walkable {
    fn walkable(&self, at: TileLoc) -> bool;
}

/// A tile is walkable when its sector is loaded, it is inside the map, and the sector's tile
/// mask does not block it. Object blocking is not decoded yet (`TODO.md` → World).
impl Walkable for LoadedMap {
    fn walkable(&self, at: TileLoc) -> bool {
        if let Some(p) = &self.properties
            && (u64::from(at.x) >= p.width || u64::from(at.y) >= p.height)
        {
            return false;
        }
        let (x, y) = at.tile_in_sector();
        self.sectors.get(&at.sector()).is_some_and(|s| s.tile_mask_blocked(x, y) != Some(true))
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
    Some(TileLoc { x: at.x.checked_add_signed(dx)?, y: at.y.checked_add_signed(dy)? })
}

/// Whether `a` and `b` are different tiles that touch (sides or corners).
pub fn adjacent(a: TileLoc, b: TileLoc) -> bool {
    a != b && a.x.abs_diff(b.x) <= 1 && a.y.abs_diff(b.y) <= 1
}

/// Whether one step from `from` to `to` is allowed: the tiles touch, `to` is walkable, and a
/// diagonal step does not cut past an unwalkable tile.
pub fn step_allowed(map: &impl Walkable, from: TileLoc, to: TileLoc) -> bool {
    if !adjacent(from, to) || !map.walkable(to) {
        return false;
    }
    from.x == to.x || from.y == to.y || (map.walkable(TileLoc { x: to.x, y: from.y }) && map.walkable(TileLoc { x: from.x, y: to.y }))
}

/// The octile distance between two tiles, in step costs.
fn estimate(a: TileLoc, b: TileLoc) -> u32 {
    let (dx, dy) = (a.x.abs_diff(b.x), a.y.abs_diff(b.y));
    DIAGONAL * dx.min(dy) + STRAIGHT * dx.abs_diff(dy)
}

/// The cheapest path from `start` to `goal`: the tiles to step on, ending with `goal` and not
/// including `start` (empty when they are the same tile). Ties are broken the same way every
/// time, so the host and the client find the same path on the same map.
pub fn find_path(map: &impl Walkable, start: TileLoc, goal: TileLoc) -> Result<Vec<TileLoc>, PathError> {
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
            let step = if d.0 != 0 && d.1 != 0 { DIAGONAL } else { STRAIGHT };
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
    pub(crate) struct Grid(pub HashSet<TileLoc>, pub u32, pub u32);

    impl Grid {
        pub(crate) fn parse(rows: &[&str]) -> Self {
            let mut blocked = HashSet::new();
            for (y, row) in rows.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    if c == '#' {
                        blocked.insert(TileLoc { x: x as u32, y: y as u32 });
                    }
                }
            }
            Self(blocked, rows[0].len() as u32, rows.len() as u32)
        }
    }

    impl Walkable for Grid {
        fn walkable(&self, at: TileLoc) -> bool {
            at.x < self.1 && at.y < self.2 && !self.0.contains(&at)
        }
    }

    pub(crate) fn t(x: u32, y: u32) -> TileLoc {
        TileLoc { x, y }
    }

    fn cost(start: TileLoc, path: &[TileLoc]) -> u32 {
        let mut prev = start;
        path.iter()
            .map(|&p| {
                let c = if p.x != prev.x && p.y != prev.y { DIAGONAL } else { STRAIGHT };
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
            "...#...",
            "...#...",
            ".......",
        ]);
        let path = find_path(&map, t(0, 0), t(6, 0)).unwrap();
        // The only way is through (3, 3). No corner is cut past (3, 2), so (3, 3) is entered
        // from (2, 3) and left to (4, 3). Cheapest: (0,0) to (2,3) is 2 diagonals and 1
        // straight (38), then 2 straights (20), then (4,3) to (6,0) is 38 again: 96.
        assert_eq!(cost(t(0, 0), &path), 96);
        assert_eq!(path.len(), 8);
        let forced = path.windows(3).position(|w| w == [t(2, 3), t(3, 3), t(4, 3)]);
        assert!(forced.is_some(), "{path:?}");
        assert_valid(&map, t(0, 0), &path);
    }

    #[test]
    fn corners_are_not_cut() {
        let map = Grid::parse(&[".#", ".."]);
        assert!(!step_allowed(&map, t(0, 0), t(1, 1)), "cuts past (1, 0)");
        assert_eq!(find_path(&map, t(0, 0), t(1, 1)).unwrap(), vec![t(0, 1), t(1, 1)]);
    }

    #[test]
    fn blocked_and_unreachable_goals_fail() {
        let map = Grid::parse(&["..#..", "..#..", "..#.."]);
        assert_eq!(find_path(&map, t(0, 0), t(2, 1)), Err(PathError::GoalBlocked));
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
