//! Which map object is under the cursor: the original tests the objects under the mouse before
//! the ground (0x4f28a0, pixel test 0x4f2680), so a click on a door is a click on the door, not
//! on the tile behind it.
//!
//! A sprite is hit where its image has a visible pixel under the point; of several, the one
//! drawn on top (highest depth) wins.

use bevy::prelude::*;
use bevy::sprite::Anchor;

/// The image pixel (column, row from the top) of a sprite under world point `point`, if the point
/// is inside it. The sprite is `size` pixels, one world unit each, placed so that its `anchor`
/// (Bevy's, -0.5..0.5 with y up) is at `translation`; `flip_x` mirrors it.
pub fn sprite_pixel(
    point: Vec2,
    translation: Vec2,
    size: UVec2,
    anchor: Vec2,
    flip_x: bool,
) -> Option<UVec2> {
    let (w, h) = (size.x as f32, size.y as f32);
    let centre = translation - anchor * Vec2::new(w, h);
    let x = point.x - (centre.x - w / 2.0);
    let y = (centre.y + h / 2.0) - point.y;
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return None;
    }
    let (mut x, y) = (x as u32, y as u32);
    if flip_x {
        x = size.x - 1 - x;
    }
    Some(UVec2::new(x, y))
}

/// Whether `image` has a visible pixel at `pixel`.
pub fn opaque_at(image: &Image, pixel: UVec2) -> bool {
    image
        .get_color_at(pixel.x, pixel.y)
        .is_ok_and(|c| c.alpha() > 0.0)
}

/// Of `sprites` (entity, sprite, anchor, transform), the one drawn on top with a visible pixel at
/// world point `point`.
pub fn pick<'a>(
    point: Vec2,
    sprites: impl IntoIterator<Item = (Entity, &'a Sprite, &'a Anchor, &'a GlobalTransform)>,
    images: &Assets<Image>,
) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for (entity, sprite, anchor, transform) in sprites {
        let Some(image) = images.get(&sprite.image) else {
            continue;
        };
        let size = image.size();
        let at = transform.translation();
        let Some(pixel) = sprite_pixel(point, at.truncate(), size, anchor.0, sprite.flip_x) else {
            continue;
        };
        if opaque_at(image, pixel) && best.is_none_or(|(z, _)| at.z > z) {
            best = Some((at.z, entity));
        }
    }
    best.map(|(_, e)| e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_maps_to_the_pixel_under_it() {
        // A 10 x 20 sprite anchored at its bottom centre, at (100, 50).
        let anchor = Vec2::new(0.0, -0.5);
        let size = UVec2::new(10, 20);
        assert_eq!(
            sprite_pixel(
                Vec2::new(100.0, 50.5),
                Vec2::new(100.0, 50.0),
                size,
                anchor,
                false
            ),
            Some(UVec2::new(5, 19)),
            "the bottom row"
        );
        assert_eq!(
            sprite_pixel(
                Vec2::new(95.5, 69.5),
                Vec2::new(100.0, 50.0),
                size,
                anchor,
                false
            ),
            Some(UVec2::new(0, 0)),
            "the top-left"
        );
        assert_eq!(
            sprite_pixel(
                Vec2::new(95.5, 69.5),
                Vec2::new(100.0, 50.0),
                size,
                anchor,
                true
            ),
            Some(UVec2::new(9, 0)),
            "mirrored"
        );
        assert_eq!(
            sprite_pixel(
                Vec2::new(94.0, 60.0),
                Vec2::new(100.0, 50.0),
                size,
                anchor,
                false
            ),
            None,
            "left of it"
        );
        assert_eq!(
            sprite_pixel(
                Vec2::new(100.0, 49.0),
                Vec2::new(100.0, 50.0),
                size,
                anchor,
                false
            ),
            None,
            "below it"
        );
    }
}
