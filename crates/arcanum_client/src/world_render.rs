//! Draws the world: the ground tiles of the sectors around the local PC.
//!
//! The map's unchanged content comes from the client's own copy of the game data: the host
//! names the map ([`LocalMap`]) and places the PC ([`LocalPc`]), and the client reads the
//! sectors around the PC itself into a [`LoadedMap`] (the same reader the host uses). Each
//! ground tile becomes a sprite centred on its [`isometric`](crate::isometric) position in the
//! original's fixed 78 x 40 rectangle (the original ignores a tile art's hotspot). Walls,
//! portals and scenery are not drawn yet.
//!
//! World units are the original's screen pixels; on entering the world the camera zooms so the
//! window shows as many of them vertically as the original's 600-pixel screen.

use std::collections::{HashMap, HashSet};

use arcanum_formats::art_id::{ArtId, ArtIndex};
use arcanum_formats::map::{SECTOR_TILES as SECTOR_SIZE, SectorId, TileLoc};
use arcanum_sim::LoadedMap;
use bevy::image::{ImageFilterMode, ImageSampler};
use bevy::prelude::*;

use crate::art_loader::ArtFrames;
use crate::isometric::{ORIGINAL_SCREEN_HEIGHT, TILE_SIZE, tile_to_world};
use crate::main_menu::{AppState, MenuBackground};
use crate::settings::BASELINE_HEIGHT;
use crate::sim_bridge::{LocalMap, LocalPc};
use crate::vfs_asset::VfsResource;

pub struct WorldRenderPlugin;

impl Plugin for WorldRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), enter_world)
            .add_systems(OnExit(AppState::InGame), leave_world)
            .add_systems(Update, (load_sectors, draw_sectors, show_tiles, follow_pc).chain().run_if(in_state(AppState::InGame)));
    }
}

/// The client's copy of the map around the local PC, and what of it is drawn.
#[derive(Resource, Default)]
pub struct ClientMap {
    pub map: LoadedMap,
    /// The tile at world position (0, 0): the corner of the PC's first sector. Map positions
    /// run to millions of pixels, too large for `f32` world coordinates to stay exact.
    pub origin: Option<TileLoc>,
    /// `None` until loaded; `Err` once it failed (reported once).
    art: Option<Result<ArtIndex, String>>,
    /// Sectors whose tiles are spawned.
    pub drawn: HashSet<SectorId>,
    paths: HashMap<u32, Option<String>>,
    frames: HashMap<String, Handle<ArtFrames>>,
    /// Tiles whose art id has no path (they are left out).
    pub unresolved: usize,
}

/// A ground tile.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroundTile(pub TileLoc);

/// A sprite waiting for its `.art` to load: frame `frame` of `art`.
#[derive(Component)]
struct PendingArt {
    art: Handle<ArtFrames>,
    frame: usize,
}

fn enter_world(
    mut commands: Commands,
    mut cameras: Query<&mut Projection, With<Camera2d>>,
    mut background: Query<&mut Visibility, With<MenuBackground>>,
) {
    commands.insert_resource(ClientMap::default());
    for mut projection in &mut cameras {
        if let Projection::Orthographic(o) = &mut *projection {
            o.scale = ORIGINAL_SCREEN_HEIGHT / BASELINE_HEIGHT;
        }
    }
    for mut visibility in &mut background {
        *visibility = Visibility::Hidden;
    }
}

fn leave_world(
    mut commands: Commands,
    mut cameras: Query<(&mut Projection, &mut Transform), With<Camera2d>>,
    mut background: Query<&mut Visibility, With<MenuBackground>>,
) {
    commands.remove_resource::<ClientMap>();
    for (mut projection, mut transform) in &mut cameras {
        if let Projection::Orthographic(o) = &mut *projection {
            o.scale = 1.0;
        }
        transform.translation = Vec3::ZERO;
    }
    for mut visibility in &mut background {
        *visibility = Visibility::Inherited;
    }
}

/// Opens the host's map and reads the sectors around the PC from the client's game data.
fn load_sectors(mut client: ResMut<ClientMap>, map: Res<LocalMap>, pc: Res<LocalPc>, vfs: Option<Res<VfsResource>>) {
    let (Some(name), Some((_, at))) = (&map.0, pc.0) else { return };
    let Some(vfs) = vfs else { return };
    let Ok(vfs) = vfs.0.read() else { return };
    if client.map.name.as_deref() != Some(name.as_str()) {
        client.map.open(Some(&vfs), name);
        client.drawn.clear();
        let corner = at.sector();
        client.origin = Some(TileLoc { x: corner.x() * SECTOR_SIZE, y: corner.y() * SECTOR_SIZE });
    }
    if client.art.is_none() {
        let art = ArtIndex::load(|p| vfs.read(p).ok()).map_err(|e| e.to_string());
        if let Err(e) = &art {
            warn!("no art index, so the world cannot be drawn: {e}");
        }
        client.art = Some(art);
    }
    let before = (client.map.sectors.len(), client.map.errors.len());
    client.map.load_around(Some(&vfs), at);
    for e in &client.map.errors[before.1..] {
        warn!("map: {e}");
    }
}

/// Spawns a sprite for every ground tile of each loaded sector not yet drawn.
fn draw_sectors(mut commands: Commands, mut client: ResMut<ClientMap>, assets: Res<AssetServer>) {
    let client = &mut *client;
    let (Some(Ok(art)), Some(origin)) = (&client.art, client.origin) else { return };
    let new: Vec<SectorId> = client.map.sectors.keys().filter(|id| !client.drawn.contains(id)).copied().collect();
    for id in new {
        let sector = &client.map.sectors[&id];
        let (base_x, base_y) = (id.x() * SECTOR_SIZE, id.y() * SECTOR_SIZE);
        for y in 0..SECTOR_SIZE {
            for x in 0..SECTOR_SIZE {
                let tile = sector.tile(x, y);
                let path = client.paths.entry(tile.0).or_insert_with(|| art.path(tile.art_id())).clone();
                let Some(path) = path else {
                    client.unresolved += 1;
                    continue;
                };
                let handle = client.frames.entry(path.clone()).or_insert_with(|| assets.load(path)).clone();
                let loc = TileLoc { x: base_x + x, y: base_y + y };
                let frame = if tile.is_facade() { facade_frame(tile.art_id()) } else { 0 };
                commands.spawn((
                    GroundTile(loc),
                    Sprite { flip_x: tile.is_mirrored(), custom_size: Some(TILE_SIZE), ..default() },
                    Transform::from_translation(tile_to_world(loc, origin).extend(0.0)),
                    Visibility::Hidden,
                    PendingArt { art: handle, frame },
                    DespawnOnExit(AppState::InGame),
                ));
            }
        }
        client.drawn.insert(id);
    }
}

/// The frame of a facade's `.art` that a facade tile shows: bits 1..10 (0x504a90).
pub fn facade_frame(id: ArtId) -> usize {
    ((id.0 >> 1) & 0x3ff) as usize
}

/// Gives each waiting sprite its frame once the `.art` has loaded; drops the ones whose art
/// failed to load.
fn show_tiles(
    mut commands: Commands,
    frames: Res<Assets<ArtFrames>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut pending: Query<(Entity, &PendingArt, &mut Sprite, &mut Visibility)>,
) {
    for (entity, wait, mut sprite, mut visibility) in &mut pending {
        if let Some(art) = frames.get(&wait.art) {
            if let Some(image) = art.frame(wait.frame) {
                // Nearest sampling: linear filtering blends the colour under transparent pixels
                // into the tile edges, a visible line round every diamond.
                if images.get(image).is_some_and(|i| !matches!(i.sampler, ImageSampler::Descriptor(ref d) if d.mag_filter == ImageFilterMode::Nearest))
                    && let Some(mut i) = images.get_mut(image)
                {
                    i.sampler = ImageSampler::nearest();
                }
                sprite.image = image.clone();
                *visibility = Visibility::Inherited;
                commands.entity(entity).remove::<PendingArt>();
            }
        } else if matches!(assets.get_load_state(wait.art.id()), Some(bevy::asset::LoadState::Failed(_))) {
            commands.entity(entity).despawn();
        }
    }
}

/// Centres the camera on the local PC.
fn follow_pc(pc: Res<LocalPc>, client: Res<ClientMap>, mut cameras: Query<&mut Transform, With<Camera2d>>) {
    let (Some((_, at)), Some(origin)) = (pc.0, client.origin) else { return };
    let target = tile_to_world(at, origin);
    for mut transform in &mut cameras {
        transform.translation.x = target.x;
        transform.translation.y = target.y;
    }
}
