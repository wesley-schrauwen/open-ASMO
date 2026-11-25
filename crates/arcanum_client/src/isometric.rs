//! Where tiles go on screen: the original's isometric transform.
//!
//! The original (`loc_to_xy`, 0x4b8680, isometric mode) puts tile `(x, y)` at screen
//! `(40 * (y - x - 1), 20 * (x + y))` plus the view origin, screen y pointing down: more `y`
//! goes right and down, more `x` left and down. A ground tile is drawn in the 78 x 40 rectangle
//! whose top-left is one pixel right of that point (tile loop 0x4d6950), so its centre is
//! `(40 * (y - x), 20 * (x + y) + 20)`; objects are placed relative to the same centre (0x4de290).
//! World positions here are tile centres in the original's pixels, y up, relative to an origin
//! tile (map positions are too large for `f32` to keep exact).

use arcanum_formats::map::TileLoc;
use bevy::prelude::*;

/// The original's screen height in pixels; the world camera shows this many world units.
pub const ORIGINAL_SCREEN_HEIGHT: f32 = 600.0;

/// Screen pixels per tile step: half a tile's width and height.
pub const HALF_TILE: IVec2 = IVec2::new(40, 20);

/// The rectangle a ground tile is drawn in (the art is scaled to it if its size differs).
pub const TILE_SIZE: Vec2 = Vec2::new(78.0, 40.0);

/// The world position of the centre of tile `at`, relative to the centre of tile `origin`.
pub fn tile_to_world(at: TileLoc, origin: TileLoc) -> Vec2 {
    let dx = i64::from(at.x) - i64::from(origin.x);
    let dy = i64::from(at.y) - i64::from(origin.y);
    Vec2::new(
        (i64::from(HALF_TILE.x) * (dy - dx)) as f32,
        -(i64::from(HALF_TILE.y) * (dx + dy)) as f32,
    )
}

/// The tile whose diamond holds world position `world` (relative to the centre of tile
/// `origin`), or `None` off the map's edge. As the original's `xy_to_loc` (0x4b8730, isometric
/// mode): with `u`, `v` the screen offset from the view origin (y down), `x = floor((v - u/2) /
/// 40)` and `y = floor((v + u/2) / 40)`. The diamond is 80 x 40 round the tile centre, each axis
/// half-open: its left and top corners and the edges between them belong to it, the others to
/// the neighbours.
pub fn world_to_tile(world: Vec2, origin: TileLoc) -> Option<TileLoc> {
    // Inverse of `tile_to_world`, in tiles from the origin's centre: dy - dx = x / 40,
    // dx + dy = -y / 20. The centre is half a tile into each of the original's intervals.
    let across = f64::from(world.x) / f64::from(HALF_TILE.x);
    let down = -f64::from(world.y) / f64::from(HALF_TILE.y);
    let dx = ((down - across) / 2.0 + 0.5).floor() as i64;
    let dy = ((down + across) / 2.0 + 0.5).floor() as i64;
    let x = u32::try_from(i64::from(origin.x) + dx).ok()?;
    let y = u32::try_from(i64::from(origin.y) + dy).ok()?;
    Some(TileLoc { x, y })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(x: u32, y: u32) -> TileLoc {
        TileLoc { x, y }
    }

    #[test]
    fn more_y_goes_right_and_down_more_x_left_and_down() {
        let o = t(100, 100);
        assert_eq!(tile_to_world(o, o), Vec2::ZERO);
        assert_eq!(tile_to_world(t(100, 101), o), Vec2::new(40.0, -20.0));
        assert_eq!(tile_to_world(t(101, 100), o), Vec2::new(-40.0, -20.0));
        assert_eq!(
            tile_to_world(t(101, 101), o),
            Vec2::new(0.0, -40.0),
            "one row down"
        );
        assert_eq!(tile_to_world(t(99, 99), o), Vec2::new(0.0, 40.0));
    }

    #[test]
    fn a_point_picks_the_tile_whose_diamond_holds_it() {
        let o = t(100, 100);
        for at in [o, t(100, 101), t(101, 100), t(99, 99), t(140, 70)] {
            let c = tile_to_world(at, o);
            assert_eq!(world_to_tile(c, o), Some(at), "centre of {at:?}");
            // Just inside each corner of the diamond.
            for d in [
                Vec2::new(39.0, 0.0),
                Vec2::new(-39.0, 0.0),
                Vec2::new(0.0, 19.0),
                Vec2::new(0.0, -19.0),
            ] {
                assert_eq!(world_to_tile(c + d, o), Some(at), "{at:?} + {d:?}");
            }
        }
        assert_eq!(
            world_to_tile(Vec2::new(21.0, -11.0), o),
            Some(t(100, 101)),
            "past the lower-right edge"
        );
        // On the edges: up-left and up-right belong to the tile, down-left and down-right not.
        assert_eq!(world_to_tile(Vec2::new(-20.0, 10.0), o), Some(o));
        assert_eq!(world_to_tile(Vec2::new(20.0, 10.0), o), Some(o));
        assert_eq!(world_to_tile(Vec2::new(20.0, -10.0), o), Some(t(100, 101)));
        assert_eq!(world_to_tile(Vec2::new(-20.0, -10.0), o), Some(t(101, 100)));
        assert_eq!(
            world_to_tile(Vec2::new(0.0, -10_000.0), t(0, 0)),
            Some(t(250, 250))
        );
        assert_eq!(
            world_to_tile(Vec2::new(0.0, 100.0), t(0, 0)),
            None,
            "off the map"
        );
    }

    #[test]
    fn far_tiles_stay_exact() {
        let o = t(92_928, 82_560);
        assert_eq!(
            tile_to_world(t(92_958, 82_592), o),
            Vec2::new(40.0 * 2.0, -20.0 * 62.0)
        );
    }
}
