//! The game's mouse cursor: interface art 0 (`cursor.art`, 24x27), set for the whole game at
//! start (Arcanum.exe 0x4013fa through `tig_mouse_set_cursor`, 0x4fffe0).
//!
//! The hotspot is the arrow's tip, pixel (0, 0). The art's centre is (0, 3), three rows below the
//! drawn tip; used as the hotspot it put clicks below the point the player sees.
//!
//! Size (owner's call, 2026-11-21): 30% of the original's share of the screen (art size times the
//! window's height over 600), so the arrow is near the system cursor's size. The art is resampled
//! with area averaging; at that size whole-pixel scaling is not possible.
//!
//! While a skill is armed ([`crate::skill_use::SkillTargeting`]) the arrow shows blue: the
//! original's targeting cursor (UI mode 0x10, art picked by 0x5719f0) was not decoded, and this
//! tint stands in for it, as the owner remembers it.
//!
//! In combat mode or a fight ([`crate::combat_ui::CombatView::attacks`]) the cursor is the battle
//! cursor (interface art 353, `battlecur.art`), and over an NPC `hoverattack.art` (555); their
//! hotspot is the art's own (builder's call: the original's mouse hotspots were not decoded).
//!
//! Not built: the cursors for actions and objects (0x573640 reads an object's cursor art from
//! its field 0x5d), and the scroll arrows (498, 499).

use bevy::prelude::*;
use bevy::window::{CursorIcon, CustomCursor, CustomCursorImage, PrimaryWindow};

use crate::art_loader::ArtFrames;
use crate::resource_pack::paths::interface;

/// The cursor's size as a share of the original's (art size times window height over 600).
pub const CURSOR_SIZE: f32 = 0.3;

/// The arrow's tip in the art, in art pixels.
const TIP: (u32, u32) = (0, 0);

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_cursor).add_systems(Update, set_cursor);
    }
}

/// Which cursor shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorKind {
    #[default]
    Arrow,
    /// Combat mode or a fight (art 353).
    Battle,
    /// The same over an NPC (art 555).
    Attack,
}

/// `battlecur.art` and `hoverattack.art`.
pub const BATTLE_CURSOR: &str = "art/interface/battlecur.art";
pub const ATTACK_CURSOR: &str = "art/interface/hoverattack.art";

/// The cursors' art, and the image size and kind its window cursor was made at.
#[derive(Resource)]
struct GameCursor {
    art: Handle<ArtFrames>,
    battle: Handle<ArtFrames>,
    attack: Handle<ArtFrames>,
    size: UVec2,
    /// Whether the window cursor was made blue.
    blue: bool,
    kind: CursorKind,
}

fn load_cursor(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(GameCursor { art: assets.load(interface::CURSOR), battle: assets.load(BATTLE_CURSOR), attack: assets.load(ATTACK_CURSOR), size: UVec2::ZERO, blue: false, kind: CursorKind::Arrow });
}

/// The cursor for the fight's state: the arrow out of combat mode and fights.
pub fn cursor_kind(view: Option<&crate::combat_ui::CombatView>, pc: Option<arcanum_sim::NetId>) -> CursorKind {
    match view {
        Some(v) if v.attacks(pc) && v.over_npc => CursorKind::Attack,
        Some(v) if v.attacks(pc) => CursorKind::Battle,
        _ => CursorKind::Arrow,
    }
}

/// Scale from art pixels to cursor pixels for a window `height` logical pixels tall.
pub fn cursor_factor(height: f32) -> f32 {
    height / 600.0 * CURSOR_SIZE
}

/// The size of a `w` x `h` image scaled by `factor`, at least 1x1.
pub fn scaled_size(w: u32, h: u32, factor: f32) -> UVec2 {
    UVec2::new(((w as f32 * factor).round() as u32).max(1), ((h as f32 * factor).round() as u32).max(1))
}

/// `image` (RGBA8) resampled to `size`. Each output pixel averages a 4x4 grid of source samples
/// with premultiplied alpha, so transparent pixels do not darken the edges. Whole-number
/// enlargements repeat each pixel exactly.
pub fn resample(image: &Image, size: UVec2) -> Option<Image> {
    const N: u32 = 4;
    let (w, h) = (image.width(), image.height());
    let data = image.data.as_ref()?;
    let (sx, sy) = (w as f32 / size.x as f32, h as f32 / size.y as f32);
    let mut out = Vec::with_capacity((size.x * size.y * 4) as usize);
    for y in 0..size.y {
        for x in 0..size.x {
            let mut acc = [0.0f32; 4];
            for j in 0..N {
                for i in 0..N {
                    let px = (((x as f32 + (i as f32 + 0.5) / N as f32) * sx) as u32).min(w - 1);
                    let py = (((y as f32 + (j as f32 + 0.5) / N as f32) * sy) as u32).min(h - 1);
                    let k = ((py * w + px) * 4) as usize;
                    let p = data.get(k..k + 4)?;
                    let a = p[3] as f32 / 255.0;
                    for c in 0..3 {
                        acc[c] += p[c] as f32 * a;
                    }
                    acc[3] += a;
                }
            }
            let a = acc[3] / (N * N) as f32;
            for c in 0..3 {
                out.push(if acc[3] > 0.0 { (acc[c] / acc[3]).round() as u8 } else { 0 });
            }
            out.push((a * 255.0).round() as u8);
        }
    }
    Some(Image::new(
        bevy::render::render_resource::Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        out,
        image.texture_descriptor.format,
        bevy::asset::RenderAssetUsages::default(),
    ))
}

/// Sets the cursor once its art is in, and again when the window's height changes its size.
fn set_cursor(
    mut commands: Commands,
    mut cursor: ResMut<GameCursor>,
    frames: Res<Assets<ArtFrames>>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    armed: Option<Res<crate::skill_use::SkillTargeting>>,
    (view, local): (Option<Res<crate::combat_ui::CombatView>>, Option<Res<crate::sim_bridge::LocalPc>>),
) {
    let Ok((entity, window)) = windows.single() else { return };
    let blue = armed.is_some_and(|a| a.0.is_some());
    let mut kind = cursor_kind(view.as_deref(), local.and_then(|l| l.0.map(|(id, _)| id)));
    // A battle cursor whose art is not in (yet) leaves the arrow.
    let handle = match kind {
        CursorKind::Arrow => &cursor.art,
        CursorKind::Battle => &cursor.battle,
        CursorKind::Attack => &cursor.attack,
    };
    let art = match frames.get(handle) {
        Some(a) => a,
        None => {
            kind = CursorKind::Arrow;
            let Some(a) = frames.get(&cursor.art) else { return };
            a
        }
    };
    let Some(frame) = art.frames.first() else { return };
    let Some(source) = images.get(frame) else { return };
    let factor = cursor_factor(window.height());
    let size = scaled_size(source.width(), source.height(), factor);
    if size == cursor.size && blue == cursor.blue && kind == cursor.kind {
        return;
    }
    let tip = match kind {
        CursorKind::Arrow => UVec2::new(TIP.0, TIP.1),
        _ => art.centers.first().map_or(UVec2::ZERO, |c| c.max(IVec2::ZERO).as_uvec2()),
    };
    let Some(mut scaled) = resample(source, size) else { return };
    if blue {
        tint_blue(&mut scaled);
    }
    let hotspot = (
        ((tip.x as f32 * factor) as u32).min(size.x - 1) as u16,
        ((tip.y as f32 * factor) as u32).min(size.y - 1) as u16,
    );
    let handle = images.add(scaled);
    commands.entity(entity).insert(CursorIcon::Custom(CustomCursor::Image(CustomCursorImage { handle, hotspot, ..default() })));
    cursor.size = size;
    cursor.blue = blue;
    cursor.kind = kind;
}

/// Turns an RGBA8 image's colours to blues of the same brightness.
pub fn tint_blue(image: &mut Image) {
    let Some(data) = image.data.as_mut() else { return };
    for px in data.as_chunks_mut::<4>().0 {
        let light = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) / 3;
        px[0] = (light / 4) as u8;
        px[1] = (light / 2) as u8;
        px[2] = (light + 64).min(255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, data: Vec<u8>) -> Image {
        Image::new(
            bevy::render::render_resource::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            data,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        )
    }

    #[test]
    fn the_cursor_is_30_percent_of_the_original_share_of_the_screen() {
        assert_eq!(scaled_size(24, 27, cursor_factor(600.0)), UVec2::new(7, 8));
        assert_eq!(scaled_size(24, 27, cursor_factor(1080.0)), UVec2::new(13, 15));
        assert_eq!(scaled_size(24, 27, cursor_factor(2160.0)), UVec2::new(26, 29));
        assert_eq!(scaled_size(24, 27, 0.0), UVec2::new(1, 1));
    }

    #[test]
    fn whole_number_enlargements_repeat_each_pixel() {
        let big = resample(&image(2, 1, vec![1, 2, 3, 255, 5, 6, 7, 255]), UVec2::new(4, 2)).unwrap();
        let row = [1, 2, 3, 255, 1, 2, 3, 255, 5, 6, 7, 255, 5, 6, 7, 255];
        assert_eq!(big.data.unwrap(), [row, row].concat());
    }

    #[test]
    fn shrinking_averages_and_transparent_pixels_do_not_darken() {
        // A white opaque pixel next to a black transparent one: half coverage, still white.
        let small = resample(&image(2, 1, vec![255, 255, 255, 255, 0, 0, 0, 0]), UVec2::new(1, 1)).unwrap();
        assert_eq!(small.data.unwrap(), vec![255, 255, 255, 128]);
    }
}
