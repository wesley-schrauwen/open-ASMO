//! The game's mouse cursor: interface art 0 (`cursor.art`, 24x27), set for the whole game at
//! start (Arcanum.exe 0x4013fa through `tig_mouse_set_cursor`, 0x4fffe0). Its hotspot is the
//! art's centre, (0, 3): the arrow's tip.
//!
//! The art is scaled up by whole pixels to the nearest of the window's height over 600 (the
//! original's screen), so it stays as sharp as the rest of the original art.
//!
//! Not built: the cursors for actions and objects (0x573640 reads an object's cursor art from
//! its field 0x5d), and the scroll arrows (498, 499).

use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};

use crate::art_loader::ArtFrames;
use crate::resource_pack::paths::interface;

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_cursor).add_systems(Update, set_cursor);
    }
}

/// The cursor's art, and the scale its window image was made at.
#[derive(Resource)]
struct GameCursor {
    art: Handle<ArtFrames>,
    scale: u32,
}

fn load_cursor(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(GameCursor { art: assets.load(interface::CURSOR), scale: 0 });
}

/// Whole-pixel scale for a window `height` logical pixels tall: the nearest to `height / 600`,
/// at least 1.
pub fn cursor_scale(height: f32) -> u32 {
    ((height / 600.0).round() as u32).max(1)
}

/// `image` (RGBA8) `scale` times larger, each pixel repeated.
pub fn scale_up(image: &Image, scale: u32) -> Option<Image> {
    let (w, h) = (image.width(), image.height());
    let data = image.data.as_ref()?;
    let mut out = Vec::with_capacity(data.len() * (scale * scale) as usize);
    for y in 0..h * scale {
        for x in 0..w * scale {
            let i = (((y / scale) * w + x / scale) * 4) as usize;
            out.extend_from_slice(data.get(i..i + 4)?);
        }
    }
    Some(Image::new(
        bevy::render::render_resource::Extent3d { width: w * scale, height: h * scale, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        out,
        image.texture_descriptor.format,
        bevy::asset::RenderAssetUsages::default(),
    ))
}

/// Sets the cursor once its art is in, and again when the window's height changes the scale.
fn set_cursor(
    mut commands: Commands,
    mut cursor: ResMut<GameCursor>,
    frames: Res<Assets<ArtFrames>>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
) {
    let Ok((entity, window)) = windows.single() else { return };
    let scale = cursor_scale(window.height());
    if scale == cursor.scale {
        return;
    }
    let Some(art) = frames.get(&cursor.art) else { return };
    let (Some(frame), Some(&center)) = (art.frames.first(), art.centers.first()) else { return };
    let Some(scaled) = images.get(frame).and_then(|image| scale_up(image, scale)) else { return };
    let (max_x, max_y) = (scaled.width().saturating_sub(1), scaled.height().saturating_sub(1));
    let hotspot = ((center.x.max(0) as u32 * scale).min(max_x) as u16, (center.y.max(0) as u32 * scale).min(max_y) as u16);
    let handle = images.add(scaled);
    commands.entity(entity).insert(CursorIcon::Custom(CustomCursor::Image(CustomCursorImage { handle, hotspot, ..default() })));
    cursor.scale = scale;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_scales_by_whole_pixels() {
        assert_eq!(cursor_scale(600.0), 1);
        assert_eq!(cursor_scale(1080.0), 2);
        assert_eq!(cursor_scale(1440.0), 2);
        assert_eq!(cursor_scale(2160.0), 4);
        assert_eq!(cursor_scale(300.0), 1);
    }

    #[test]
    fn scaling_repeats_each_pixel() {
        let image = Image::new(
            bevy::render::render_resource::Extent3d { width: 2, height: 1, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            vec![1, 2, 3, 4, 5, 6, 7, 8],
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        );
        let big = scale_up(&image, 2).unwrap();
        assert_eq!((big.width(), big.height()), (4, 2));
        let row = [1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8, 5, 6, 7, 8];
        assert_eq!(big.data.unwrap(), [row, row].concat());
    }
}
